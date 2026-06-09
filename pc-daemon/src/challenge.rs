//! Challenge-Response 质询响应认证模块

use anyhow::{Context, Result};
use btleplug::api::{CharPropFlags, Peripheral as _, WriteType};
use futures::StreamExt;
use log::{info, warn};
use rand::RngCore;
use serde_json::json;
use shadowgate_core::crypto::{PublicKey, SignatureBytes};
use shadowgate_core::protocol;
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::ble_scanner::DiscoveredDevice;
use crate::ipc::program_data_dir;
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
        info!(
            "Starting Challenge-Response for device hash {:02x?}",
            device.device_info.device_hash
        );

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

        if response_char.properties.contains(CharPropFlags::NOTIFY) {
            peripheral.subscribe(response_char).await?;
        }

        let mut challenge = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut challenge);

        let write_type = if challenge_char
            .properties
            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        {
            WriteType::WithoutResponse
        } else {
            WriteType::WithResponse
        };

        peripheral
            .write(challenge_char, &challenge, write_type)
            .await?;
        info!("Challenge sent");

        let timeout_ms = self.config.challenge.timeout_ms;
        let result = timeout(Duration::from_millis(timeout_ms), async {
            let mut stream = peripheral.notifications().await?;
            loop {
                match stream.next().await {
                    Some(notification) if notification.uuid == response_uuid => {
                        return Ok(notification.value);
                    }
                    Some(_) => {}
                    None => anyhow::bail!("notification stream ended before response"),
                }
            }
        })
        .await;

        let _ = peripheral.unsubscribe(response_char).await;
        let _ = peripheral.disconnect().await;

        match result {
            Ok(Ok(data)) => {
                info!("Response received, verifying...");
                self.verify_response(&device.device_info.device_hash, &challenge, &data)
                    .await
            }
            Ok(Err(e)) => {
                warn!("Challenge failed: {}", e);
                Ok(false)
            }
            Err(_) => {
                warn!("Challenge timeout");
                Ok(false)
            }
        }
    }

    async fn verify_response(
        &self,
        device_hash: &[u8; 8],
        challenge: &[u8; 32],
        response_data: &[u8],
    ) -> Result<bool> {
        let resp: protocol::ChallengeResponse = protocol::deserialize(response_data)?;

        let mut ctx = self.ctx.lock().await;
        let device_key = ctx.device_store.get_by_hash(device_hash).copied();

        let pk = match device_key {
            Some(key) => PublicKey::from_bytes(key)?,
            None => return Ok(false),
        };

        let sig = SignatureBytes {
            bytes: resp.signature.clone(),
        };
        let valid = pk
            .verify(challenge, &sig)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if valid {
            ctx.device_store.mark_authenticated(device_hash);
            write_credential_authorization(device_hash, self.config.challenge.timeout_ms);
        }

        Ok(valid)
    }
}

fn write_credential_authorization(device_hash: &[u8; 8], challenge_timeout_ms: u64) {
    let mut nonce = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let payload = json!({
        "authorized_until_ms": now_ms + challenge_timeout_ms + 750,
        "device_hash": hex::encode(device_hash),
        "auth_nonce": hex::encode(nonce),
    });

    let path = program_data_dir().join("credential_auth.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        path,
        serde_json::to_vec_pretty(&payload).unwrap_or_default(),
    );
}
