//! Challenge-Response 质询响应认证模块
//!
//! 流程:
//! 1. PC 连接 BLE 设备
//! 2. 发现 ShadowGate 服务和特性
//! 3. 生成 32 字节随机质询
//! 4. 写入 Challenge Characteristic
//! 5. 等待 Response Characteristic 通知
//! 6. 用已存公钥验证 Ed25519 签名
//! 7. 超时检测 (防范中继攻击)

use anyhow::{Context, Result};
use btleplug::api::{CharPropFlags, Peripheral as _, WriteType};
use log::{debug, error, info, warn};
use rand::RngCore;
use shadowgate_core::crypto::{PublicKey, SignatureBytes};
use shadowgate_core::protocol;
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout, Duration};
use uuid::Uuid;

use crate::ble_scanner::DiscoveredDevice;
use crate::device_store::DeviceStore;
use crate::state_machine::AppContext;

/// Challenge-Response 认证执行器
pub struct ChallengeRunner {
    config: ShadowGateConfig,
    ctx: Arc<Mutex<AppContext>>,
}

impl ChallengeRunner {
    pub fn new(config: ShadowGateConfig, ctx: Arc<Mutex<AppContext>>) -> Self {
        ChallengeRunner { config, ctx }
    }

    /// 执行完整的 Challenge-Response 认证流程
    ///
    /// 返回: true = 认证通过, false = 认证失败
    pub async fn run(&self, device: DiscoveredDevice) -> Result<bool> {
        info!("Starting Challenge-Response for device: {:?}", device.name);

        // 连接到设备
        let peripheral = &device.peripheral;

        if !peripheral.is_connected().await? {
            info!("Connecting to device...");
            peripheral
                .connect()
                .await
                .context("Failed to connect to device")?;
            info!("Connected successfully");
        }

        // 发现服务
        peripheral
            .discover_services()
            .await
            .context("Failed to discover services")?;

        let characteristics = peripheral.characteristics();

        // 解析 UUID
        let challenge_uuid: Uuid = self
            .config
            .ble
            .characteristic_challenge_uuid
            .parse()
            .context("Invalid CHALLENGE_CHARACTERISTIC_UUID")?;

        let response_uuid: Uuid = self
            .config
            .ble
            .characteristic_response_uuid
            .parse()
            .context("Invalid RESPONSE_CHARACTERISTIC_UUID")?;

        // 找到 Challenge Characteristic (PC 写入)
        let challenge_char = characteristics
            .iter()
            .find(|c| c.uuid == challenge_uuid)
            .context("Challenge Characteristic not found")?;

        // 找到 Response Characteristic (PC 读取/订阅通知)
        let response_char = characteristics
            .iter()
            .find(|c| c.uuid == response_uuid)
            .context("Response Characteristic not found")?;

        // 订阅 Response Characteristic 通知
        if response_char.properties.contains(CharPropFlags::NOTIFY) {
            peripheral
                .subscribe(&response_char)
                .await
                .context("Failed to subscribe to response characteristic")?;
        }

        // 生成随机质询
        let mut challenge = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut challenge);

        let sequence: u32 = rand::thread_rng().next_u32();

        // 序列化质询请求
        let challenge_payload = protocol::create_challenge_request(&challenge, sequence)?;

        info!(
            "Sending challenge (seq={}, size={} bytes)...",
            sequence,
            challenge_payload.len()
        );

        // 写入 Challenge (使用 Write Without Response 以节省时间)
        let write_type = if challenge_char
            .properties
            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        {
            WriteType::WithoutResponse
        } else if challenge_char.properties.contains(CharPropFlags::WRITE) {
            WriteType::WithResponse
        } else {
            anyhow::bail!("Challenge characteristic is not writable");
        };

        peripheral
            .write(&challenge_char, &challenge_payload, write_type)
            .await
            .context("Failed to write challenge")?;

        info!(
            "Challenge sent, waiting for response (timeout: {}ms)...",
            self.config.challenge.timeout_ms
        );

        // 等待响应通知 (带超时)
        let timeout_duration = Duration::from_millis(self.config.challenge.timeout_ms);

        let response_result = timeout(timeout_duration, async {
            let mut notification_stream = peripheral
                .notifications()
                .await
                .map_err(|e| anyhow::anyhow!("Notification stream error: {}", e))?;

            // 等待第一个通知
            loop {
                let notification = notification_stream
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Notification stream closed"))?;

                if notification.uuid == response_uuid {
                    debug!("Received response: {} bytes", notification.value.len());
                    return Ok(notification.value);
                }
            }
        })
        .await;

        // 取消订阅
        let _ = peripheral.unsubscribe(&response_char).await;

        // 断开连接
        let _ = peripheral.disconnect().await;

        // 处理响应
        match response_result {
            Ok(Ok(data)) => {
                info!("Response received in time");
                self.verify_response(&challenge, sequence, &data).await
            }
            Ok(Err(e)) => {
                error!("Error receiving response: {}", e);
                Ok(false)
            }
            Err(_elapsed) => {
                warn!(
                    "Challenge-Response TIMEOUT after {}ms — possible relay attack",
                    self.config.challenge.timeout_ms
                );
                Ok(false)
            }
        }
    }

    /// 验证签名响应
    async fn verify_response(
        &self,
        challenge: &[u8; 32],
        expected_sequence: u32,
        response_data: &[u8],
    ) -> Result<bool> {
        // 反序列化响应
        let resp: protocol::ChallengeResponse = protocol::deserialize(response_data)
            .context("Failed to deserialize challenge response")?;

        // 验证序列号 (防重放)
        if resp.sequence != expected_sequence {
            warn!(
                "Sequence mismatch: expected {}, got {}",
                expected_sequence, resp.sequence
            );
            return Ok(false);
        }

        // 查找设备公钥
        let device_hash = {
            let ctx = self.ctx.lock().await;
            ctx.connected_device
                .as_ref()
                .map(|d| d.device_info.device_hash)
        };

        let device_hash = match device_hash {
            Some(h) => h,
            None => {
                warn!("No connected device");
                return Ok(false);
            }
        };

        let public_key = {
            let ctx = self.ctx.lock().await;
            ctx.device_store.get_by_hash(&device_hash).cloned()
        };

        let public_key = match public_key {
            Some(pk) => pk,
            None => {
                warn!("Device not in trusted store");
                return Ok(false);
            }
        };

        // 构造待验证消息: challenge || sequence (4 bytes, big-endian)
        let mut message = Vec::with_capacity(36);
        message.extend_from_slice(challenge);
        message.extend_from_slice(&expected_sequence.to_be_bytes());

        // 验证 Ed25519 签名
        let pk = PublicKey::from_bytes(public_key).context("Invalid stored public key")?;

        let sig = SignatureBytes {
            bytes: resp.signature,
        };

        match pk.verify(&message, &sig) {
            Ok(true) => {
                info!("Signature VERIFIED successfully");
                Ok(true)
            }
            Ok(false) => {
                warn!("Signature verification FAILED — possible impersonation");
                Ok(false)
            }
            Err(e) => {
                error!("Signature verification error: {}", e);
                Ok(false)
            }
        }
    }
}
