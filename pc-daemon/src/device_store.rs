//! 信任设备存储模块
//!
//! 持久化已配对设备的公钥和哈希，存储在 JSON 文件中。
//! 存储路径: %APPDATA%\ShadowGate\trusted_devices.json

use anyhow::{Context, Result};
use log::info;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 单个信任设备记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedDevice {
    /// 设备哈希 (8 字节，用于广播匹配)
    pub device_hash: String, // hex encoded
    /// 设备名称
    pub name: String,
    /// Ed25519 公钥 (hex encoded, 64 chars)
    pub public_key_hex: String,
    /// 原始公钥字节 (hex 解码后 32 bytes)
    #[serde(skip)]
    pub public_key: [u8; 32],
    /// 配对时间 (Unix 时间戳)
    pub paired_at: u64,
    /// 最后认证时间
    pub last_auth_at: Option<u64>,
}

/// 设备存储
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStore {
    devices: Vec<TrustedDevice>,
    /// hash → index 索引加速查找
    #[serde(skip)]
    hash_index: HashMap<String, usize>,
}

impl DeviceStore {
    /// 创建空的设备存储
    pub fn new() -> Self {
        DeviceStore {
            devices: Vec::new(),
            hash_index: HashMap::new(),
        }
    }

    /// 从文件加载设备存储
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            info!("Device store not found at {:?}, creating new", path);
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(path).context("Failed to read device store file")?;

        let mut store: DeviceStore =
            serde_json::from_str(&content).context("Failed to parse device store JSON")?;

        // 从 hex 解码公钥
        for device in store.devices.iter_mut() {
            device.public_key = hex_to_bytes(&device.public_key_hex)
                .context("Invalid public key hex in device store")?;
        }

        // 重建索引
        store.rebuild_index();

        Ok(store)
    }

    /// 保存设备存储到文件
    pub fn save(&self, path: &Path) -> Result<()> {
        // 序列化前确保 public_key_hex 已填充
        let mut save_copy = self.clone();
        for device in save_copy.devices.iter_mut() {
            device.public_key_hex = bytes_to_hex(&device.public_key);
        }

        let content =
            serde_json::to_string_pretty(&save_copy).context("Failed to serialize device store")?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create device store directory")?;
        }

        std::fs::write(path, &content).context("Failed to write device store file")?;

        info!("Device store saved ({})", path.display());
        Ok(())
    }

    /// 添加信任设备
    pub fn add_device(
        &mut self,
        device_hash: [u8; 8],
        name: String,
        public_key: [u8; 32],
    ) -> Result<()> {
        let hash_hex = bytes_to_hex(&device_hash);

        // 检查是否已存在
        if self.hash_index.contains_key(&hash_hex) {
            anyhow::bail!("Device with hash {} already exists", hash_hex);
        }

        let device = TrustedDevice {
            device_hash: hash_hex.clone(),
            name,
            public_key_hex: bytes_to_hex(&public_key),
            public_key,
            paired_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            last_auth_at: None,
        };

        self.devices.push(device);
        self.hash_index.insert(hash_hex, self.devices.len() - 1);

        info!("Added trusted device");
        Ok(())
    }

    /// 移除信任设备
    pub fn remove_device(&mut self, device_hash: &[u8; 8]) -> Result<()> {
        let hash_hex = bytes_to_hex(device_hash);

        if let Some(&idx) = self.hash_index.get(&hash_hex) {
            self.devices.remove(idx);
            self.rebuild_index();
            info!("Removed trusted device: {}", hash_hex);
            Ok(())
        } else {
            anyhow::bail!("Device not found: {}", hash_hex);
        }
    }

    /// 通过哈希查找设备公钥
    pub fn get_by_hash(&self, device_hash: &[u8; 8]) -> Option<&[u8; 32]> {
        let hash_hex = bytes_to_hex(device_hash);
        self.hash_index
            .get(&hash_hex)
            .map(|&idx| &self.devices[idx].public_key)
    }

    /// 检查设备是否受信任
    pub fn is_trusted(&self, device_hash: &[u8; 8]) -> bool {
        self.get_by_hash(device_hash).is_some()
    }

    /// 获取所有设备
    pub fn all_devices(&self) -> &[TrustedDevice] {
        &self.devices
    }

    /// 设备计数
    pub fn count(&self) -> usize {
        self.devices.len()
    }

    /// 更新最后认证时间
    pub fn mark_authenticated(&mut self, device_hash: &[u8; 8]) {
        let hash_hex = bytes_to_hex(device_hash);
        if let Some(&idx) = self.hash_index.get(&hash_hex) {
            self.devices[idx].last_auth_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
        }
    }

    fn rebuild_index(&mut self) {
        self.hash_index.clear();
        for (i, device) in self.devices.iter().enumerate() {
            self.hash_index.insert(device.device_hash.clone(), i);
        }
    }
}

impl Default for DeviceStore {
    fn default() -> Self {
        Self::new()
    }
}

// ===== 工具函数 =====

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_to_bytes(hex: &str) -> Result<[u8; 32]> {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Invalid hex string")?;

    bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("Expected 32 bytes"))
}
