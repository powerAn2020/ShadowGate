//! ShadowGate Core Library
//!
//! 平台无关的核心逻辑层 —— BLE 跨端自动锁屏/解锁系统的共享协议与算法库。
//!
//! ## 模块架构
//!
//! - `crypto`: Ed25519 密钥生成、签名与验证
//! - `rssi_filter`: 卡尔曼滤波、滑动平均、迟滞防抖
//! - `protocol`: Bincode 序列化协议
//! - `config`: TOML 配置加载
//! - `ffi`: Android JNI 接口 (仅在 target_os="android" 时编译)
//! - `error`: 统一错误类型

pub mod crypto;
pub mod rssi_filter;
pub mod protocol;
pub mod config;
pub mod ffi;
pub mod error;

// Re-export commonly used types
pub use crypto::{KeyPair, PublicKey, SignatureBytes};
pub use rssi_filter::{HysteresisAction, HysteresisDetector, KalmanFilter, MovingAverageFilter};
pub use protocol::{
    ChallengeRequest, ChallengeResponse, DeviceInfo, MessageType, ProvisioningRequest,
    ProvisioningResponse, PROTOCOL_VERSION,
};
pub use config::ShadowGateConfig;
pub use error::{CoreError, CoreResult};
