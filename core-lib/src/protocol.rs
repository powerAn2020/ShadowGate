//! BLE 通信协议序列化模块
//!
//! 基于 `bincode` 实现高效二进制序列化，最小化 BLE 传输负载。
//!
//! 协议消息类型:
//! - `ProvisioningRequest`: 配对手设备公钥交换
//! - `ChallengeRequest`: PC → Android，质询随机数
//! - `ChallengeResponse`: Android → PC，签名响应
//! - `DeviceInfo`: 设备身份信息 (用于广播/扫描过滤)

use serde::{Deserialize, Serialize};

use crate::crypto::{PublicKey, SignatureBytes};
use crate::error::{CoreError, CoreResult};

/// 协议版本 (用于兼容性检查)
pub const PROTOCOL_VERSION: u8 = 1;

/// 最大 BLE 单包负载 (20 字节，标准 BLE 4.x 限制)
/// 使用 Write Long Characteristic 可扩展到 MTU-3
pub const BLE_MTU_DEFAULT: usize = 20;

/// 消息帧头
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameHeader {
    /// 协议版本
    pub version: u8,
    /// 消息类型
    pub msg_type: MessageType,
    /// 序列号 (用于防重放)
    pub sequence: u32,
    /// 有效负载长度
    pub payload_len: u16,
}

/// 消息类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MessageType {
    /// 设备信息广播 (Android → PC，通过 Advertise Data)
    DeviceInfo = 0x01,
    /// 配对手公钥交换
    ProvisioningRequest = 0x10,
    /// 配对手公钥确认
    ProvisioningResponse = 0x11,
    /// 质询请求 (PC → Android)
    ChallengeRequest = 0x20,
    /// 签名响应 (Android → PC)
    ChallengeResponse = 0x21,
    /// 心跳包
    Heartbeat = 0x30,
}

impl MessageType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::DeviceInfo),
            0x10 => Some(Self::ProvisioningRequest),
            0x11 => Some(Self::ProvisioningResponse),
            0x20 => Some(Self::ChallengeRequest),
            0x21 => Some(Self::ChallengeResponse),
            0x30 => Some(Self::Heartbeat),
            _ => None,
        }
    }
}

/// 设备信息 (嵌入在 BLE Advertise Data 中)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// 设备哈希 ID (SHA-256 前 8 字节，用于快速识别)
    pub device_hash: [u8; 8],
    /// 协议版本
    pub protocol_version: u8,
    /// 设备能力标志位
    pub capabilities: u8,
}

/// 配对手请求 — 交换公钥
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisioningRequest {
    /// 请求方公钥 (32 字节 Ed25519)
    pub public_key: [u8; 32],
    /// 设备名称 (UTF-8)
    pub device_name: String,
}

/// 配对手响应 — 回传公钥
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisioningResponse {
    /// 响应方公钥 (32 字节 Ed25519)
    pub public_key: [u8; 32],
    /// 对请求的签名 (证明持有私钥, 64 字节)
    #[serde(with = "serde_bytes")]
    pub signature: Vec<u8>,
    /// 设备名称
    pub device_name: String,
}

/// 质询请求 — PC 发送随机数给 Android
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeRequest {
    /// 32 字节随机质询
    pub challenge: [u8; 32],
    /// 序列号 (防重放)
    pub sequence: u32,
    /// 时间戳 (毫秒，用于超时检测)
    pub timestamp_ms: u64,
}

/// 质询响应 — Android 返回签名
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResponse {
    /// 签名 (64 字节 Ed25519)
    #[serde(with = "serde_bytes")]
    pub signature: Vec<u8>,
    /// 回显序列号
    pub sequence: u32,
    /// 当前设备 RSSI (Android 端测得，用于双向校准)
    pub device_rssi: i8,
}

/// 心跳包
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub timestamp_ms: u64,
    pub battery_level: u8,
}

// ===== 序列化 / 反序列化工具函数 =====

/// 序列化消息为二进制 (bincode)
pub fn serialize<T: Serialize>(msg: &T) -> CoreResult<Vec<u8>> {
    bincode::serialize(msg).map_err(CoreError::SerializationError)
}

/// 从二进制反序列化消息
pub fn deserialize<T: for<'de> Deserialize<'de>>(data: &[u8]) -> CoreResult<T> {
    bincode::deserialize(data).map_err(CoreError::SerializationError)
}

/// 创建配对手请求的 payload 并序列化
pub fn create_provisioning_request(
    public_key: &PublicKey,
    device_name: &str,
) -> CoreResult<Vec<u8>> {
    let req = ProvisioningRequest {
        public_key: public_key.bytes,
        device_name: device_name.to_string(),
    };
    serialize(&req)
}

/// 创建质询请求
pub fn create_challenge_request(challenge: &[u8; 32], sequence: u32) -> CoreResult<Vec<u8>> {
    let req = ChallengeRequest {
        challenge: *challenge,
        sequence,
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    };
    serialize(&req)
}

/// 创建质询响应
pub fn create_challenge_response(
    signature: &SignatureBytes,
    sequence: u32,
    device_rssi: i8,
) -> CoreResult<Vec<u8>> {
    let resp = ChallengeResponse {
        signature: signature.bytes.clone(),
        sequence,
        device_rssi,
    };
    serialize(&resp)
}

/// 创建设备信息 (用于 BLE Advertise Data)
pub fn create_device_info(device_hash: &[u8; 8], capabilities: u8) -> CoreResult<Vec<u8>> {
    let info = DeviceInfo {
        device_hash: *device_hash,
        protocol_version: PROTOCOL_VERSION,
        capabilities,
    };
    serialize(&info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_challenge_roundtrip() {
        let challenge = [0xABu8; 32];
        let data = create_challenge_request(&challenge, 42).unwrap();
        let deserialized: ChallengeRequest = deserialize(&data).unwrap();
        assert_eq!(deserialized.challenge, challenge);
        assert_eq!(deserialized.sequence, 42);
    }

    #[test]
    fn test_device_info_size() {
        let hash = [0u8; 8];
        let data = create_device_info(&hash, 0).unwrap();
        // DeviceInfo 应 < 20 字节以放入 BLE Advertise Data
        assert!(
            data.len() <= 20,
            "DeviceInfo too large: {} bytes",
            data.len()
        );
    }

    #[test]
    fn test_message_type_roundtrip() {
        for mt in [
            MessageType::DeviceInfo,
            MessageType::ChallengeRequest,
            MessageType::ChallengeResponse,
            MessageType::Heartbeat,
        ] {
            let decoded = MessageType::from_u8(mt as u8);
            assert_eq!(decoded, Some(mt));
        }
    }
}
