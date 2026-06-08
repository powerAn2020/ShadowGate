//! 全局配置管理模块
//!
//! 从 TOML 配置文件加载配置，所有参数外部化，禁止硬编码。

use crate::error::CoreResult;
use serde::{Deserialize, Serialize};

/// ShadowGate 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowGateConfig {
    pub ble: BleConfig,
    pub rssi: RssiConfig,
    pub challenge: ChallengeConfig,
    pub scanning: ScanningConfig,
    pub crypto: CryptoConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BleConfig {
    pub service_uuid: String,
    pub characteristic_challenge_uuid: String,
    pub characteristic_response_uuid: String,
    pub characteristic_device_id_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssiConfig {
    pub unlock_threshold_dbm: i32,
    pub lock_threshold_dbm: i32,
    pub hysteresis_samples: usize,
    pub lock_confirmation_ms: u64,
    pub rssi_sample_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeConfig {
    pub timeout_ms: u64,
    pub challenge_size: usize,
    pub response_max_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanningConfig {
    pub scan_interval_ms: u64,
    pub scan_window_ms: u64,
    pub scan_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoConfig {
    pub key_type: String,
    pub key_rotation_days: u32,
}

impl ShadowGateConfig {
    /// 从 TOML 字符串加载配置
    pub fn from_toml_str(content: &str) -> CoreResult<Self> {
        toml::from_str(content)
            .map_err(|e| crate::error::CoreError::ConfigError(format!("TOML parse error: {}", e)))
    }

    /// 从文件加载配置
    pub fn from_file(path: &std::path::Path) -> CoreResult<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml_str(&content)
    }

    /// 获取默认配置 (内置 fallback)
    pub fn default_config() -> Self {
        ShadowGateConfig {
            ble: BleConfig {
                service_uuid: "0000shadow-0000-1000-8000-00805f9b34fb".to_string(),
                characteristic_challenge_uuid: "0000chall-0000-1000-8000-00805f9b34fb".to_string(),
                characteristic_response_uuid: "0000resp-0000-1000-8000-00805f9b34fb".to_string(),
                characteristic_device_id_uuid: "0000devid-0000-1000-8000-00805f9b34fb".to_string(),
            },
            rssi: RssiConfig {
                unlock_threshold_dbm: -60,
                lock_threshold_dbm: -80,
                hysteresis_samples: 3,
                lock_confirmation_ms: 5000,
                rssi_sample_interval_ms: 500,
            },
            challenge: ChallengeConfig {
                timeout_ms: 1500,
                challenge_size: 32,
                response_max_size: 64,
            },
            scanning: ScanningConfig {
                scan_interval_ms: 1000,
                scan_window_ms: 500,
                scan_timeout_secs: 0,
            },
            crypto: CryptoConfig {
                key_type: "ed25519".to_string(),
                key_rotation_days: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = ShadowGateConfig::default_config();
        assert_eq!(cfg.rssi.unlock_threshold_dbm, -60);
        assert_eq!(cfg.rssi.lock_threshold_dbm, -80);
        assert!(
            cfg.rssi.unlock_threshold_dbm > cfg.rssi.lock_threshold_dbm,
            "hysteresis buffer requires unlock > lock"
        );
    }

    #[test]
    fn test_parse_toml() {
        let toml_str = r#"
[ble]
service_uuid = "test-uuid"
characteristic_challenge_uuid = "chall-uuid"
characteristic_response_uuid = "resp-uuid"
characteristic_device_id_uuid = "devid-uuid"

[rssi]
unlock_threshold_dbm = -55
lock_threshold_dbm = -85
hysteresis_samples = 3
lock_confirmation_ms = 5000
rssi_sample_interval_ms = 500

[challenge]
timeout_ms = 2000
challenge_size = 32
response_max_size = 64

[scanning]
scan_interval_ms = 800
scan_window_ms = 400
scan_timeout_secs = 0

[crypto]
key_type = "ed25519"
key_rotation_days = 30
"#;
        let cfg = ShadowGateConfig::from_toml_str(toml_str).unwrap();
        assert_eq!(cfg.ble.service_uuid, "test-uuid");
        assert_eq!(cfg.rssi.unlock_threshold_dbm, -55);
        assert_eq!(cfg.challenge.timeout_ms, 2000);
    }
}
