//! Challenge-Response 质询响应认证模块 (简化版)

use anyhow::{Context, Result};
use btleplug::api::{CharPropFlags, Peripheral as _, WriteType};
use log::{debug, error, info, warn};
use rand::RngCore;
use shadowgate_core::crypto::PublicKey;
use shadowgate_core::protocol;
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration};
use uuid::Uuid;

use crate::ble_scanner::DiscoveredDevice;
use crate::state_machine::AppContext;

pub struct ChallengeRunner {
    config: ShadowGateConfig,
    ctx: Arc<Mutex<AppContext>>,
}

impl ChallengeRunner {
    pub fn new(config: ShadowGateConfig, ctx: Arc<Mutex<AppContext>>) -> Self {
        ChallengeRunner { config, ctx }
    }

    pub async fn run(&self, device: DiscoveredDevice) -> Result<bool> {
        info!("Starting Challenge-Response...");

        let peripheral = &device.peripheral;
        if !peripheral.is_connected().await? {
            peripheral.connect().await.context("Connect failed")?;
        }

        peripheral.discover_services().await?;
        let characteristics = peripheral.characteristics();

        let challenge_uuid: Uuid = self.config.ble.characteristic_challenge_uuid.parse()?;
        let response_uuid: Uuid = self.config.ble.characteristic_response_uuid.parse()?;

        let challenge_char = characteristics
            .iter()
            .find(|c| c.uuid == challenge_uuid)
            .context("Challenge char not found")?;

        let response_char = characteristics
            .iter()
            .find(|c| c.uuid == response_uuid)
            .context("Response char not found")?;

        // Subscribe to notifications
        if response_char.properties.contains(CharPropFlags::NOTIFY) {
            peripheral.subscribe(&response_char).await?;
        }

        // Generate challenge
        let mut challenge = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut challenge);
        let sequence: u32 = rand::thread_rng().next_u32();

        let challenge_payload = protocol::create_challenge_request(&challenge, sequence)?;

        let write_type = if challenge_char
            .properties
            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        {
            WriteType::WithoutResponse
        } else {
            WriteType::WithResponse
        };

        peripheral
            .write(&challenge_char, &challenge_payload, write_type)
            .await?;
        info!("Challenge sent (seq={})", sequence);

        // Wait for notification with timeout
        let timeout_ms = self.config.challenge.timeout_ms;
        let result = timeout(Duration::from_millis(timeout_ms), async {
            let mut stream = peripheral.notifications().await?;
            loop {
                if let Some(notification) = stream.next().await {
                    if notification.uuid == response_uuid {
                        return Ok(notification.value);
                    }
                }
            }
        })
        .await;

        let _ = peripheral.unsubscribe(&response_char).await;
        let _ = peripheral.disconnect().await;

        match result {
            Ok(Ok(data)) => {
                info!("Response received, verifying...");
                self.verify_response(&challenge, sequence, &data).await
            }
            _ => {
                warn!("Challenge timeout");
                Ok(false)
            }
        }
    }

    async fn verify_response(
        &self,
        challenge: &[u8; 32],
        sequence: u32,
        response_data: &[u8],
    ) -> Result<bool> {
        let resp: protocol::ChallengeResponse = protocol::deserialize(response_data)?;

        if resp.sequence != sequence {
            return Ok(false);
        }

        let ctx = self.ctx.lock().await;
        let device_key = ctx.device_store.all_devices().first().map(|d| d.public_key);

        let pk = match device_key {
            Some(key) => PublicKey::from_bytes(key)?,
            None => return Ok(false),
        };

        let mut message = Vec::with_capacity(36);
        message.extend_from_slice(challenge);
        message.extend_from_slice(&sequence.to_be_bytes());

        let sig = shadowgate_core::crypto::SignatureBytes {
            bytes: resp.signature.clone(),
        };

        pk.verify(&message, &sig)
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
}
