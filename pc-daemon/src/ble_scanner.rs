//! BLE 扫描器模块
//!
//! 使用 btleplug 库异步扫描 BLE 设备，匹配服务 UUID 并解析广播数据。

use anyhow::{Context, Result};
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use log::{debug, info, warn};
use shadowgate_core::protocol::{self, DeviceInfo};
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

/// 发现的设备信息
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// BLE 外设对象
    pub peripheral: btleplug::platform::Peripheral,
    /// 解析后的设备信息
    pub device_info: DeviceInfo,
    /// 当前 RSSI (dBm)
    pub rssi: i16,
    /// 设备名称
    pub name: Option<String>,
}

/// BLE 扫描器
pub struct BleScanner {
    adapter: Adapter,
    config: ShadowGateConfig,
}

impl BleScanner {
    /// 初始化 BLE 扫描器
    pub async fn new(config: ShadowGateConfig) -> Result<Self> {
        let manager = Manager::new().await.context("Failed to create BLE manager")?;

        let adapters = manager
            .adapters()
            .await
            .context("Failed to enumerate BLE adapters")?;

        if adapters.is_empty() {
            anyhow::bail!("No BLE adapter found. Is Bluetooth enabled?");
        }

        let adapter = adapters
            .into_iter()
            .next()
            .context("No BLE adapter available")?;

        info!(
            "Using BLE adapter: {:?}",
            adapter.adapter_info().await.unwrap_or_default()
        );

        Ok(BleScanner { adapter, config })
    }

    /// 获取适配器引用 (用于重置等操作)
    pub fn adapter(&self) -> &Adapter {
        &self.adapter
    }

    /// 启动扫描，返回异步流式发现的设备
    ///
    /// 此方法会持续扫描，调用回调处理每个发现的设备。
    /// 当回调返回 `true` 时，停止扫描。
    pub async fn scan<F>(&mut self, filter_uuid: &Uuid, mut on_device: F) -> Result<()>
    where
        F: FnMut(DiscoveredDevice) -> bool,
    {
        self.adapter
            .start_scan(ScanFilter::default())
            .await
            .context("Failed to start BLE scan")?;

        info!("BLE scan started (filter: {})", filter_uuid);

        loop {
            let peripherals = self.adapter.peripherals().await?;

            for p in peripherals.iter() {
                let properties = match p.properties().await? {
                    Some(props) => props,
                    None => continue,
                };

                // 检查服务 UUID
                let has_service = properties
                    .services
                    .iter()
                    .any(|s| s == filter_uuid);

                if !has_service {
                    continue;
                }

                // 提取 RSSI
                let rssi = match properties.rssi {
                    Some(r) => r,
                    None => {
                        debug!("No RSSI for device: {:?}", properties.local_name);
                        continue;
                    }
                };

                // 尝试解析 DeviceInfo 从制造商数据或服务数据
                let device_info = parse_device_info(&properties);

                if let Some(info) = device_info {
                    let discovered = DiscoveredDevice {
                        peripheral: p.clone(),
                        device_info: info,
                        rssi,
                        name: properties.local_name.clone(),
                    };

                    debug!(
                        "Discovered device: {:?} (RSSI: {} dBm, hash: {:02x?})",
                        discovered.name, discovered.rssi, discovered.device_info.device_hash
                    );

                    if on_device(discovered) {
                        // 回调返回 true，停止扫描
                        self.adapter.stop_scan().await.ok();
                        return Ok(());
                    }
                }
            }

            // 扫描间隔
            sleep(Duration::from_millis(self.config.scanning.scan_interval_ms)).await;
        }
    }

    /// 停止扫描
    pub async fn stop_scan(&self) -> Result<()> {
        self.adapter
            .stop_scan()
            .await
            .context("Failed to stop BLE scan")?;
        info!("BLE scan stopped");
        Ok(())
    }

    /// 重置适配器 (用于休眠唤醒后恢复)
    pub async fn reset(&mut self) -> Result<()> {
        info!("Resetting BLE adapter...");
        // 停止扫描、短暂关闭、重新初始化
        let _ = self.adapter.stop_scan().await;

        let manager = Manager::new().await?;
        let adapters = manager.adapters().await?;
        if let Some(new_adapter) = adapters.into_iter().next() {
            self.adapter = new_adapter;
        }
        info!("BLE adapter reset complete");
        Ok(())
    }
}

/// 从广播属性中解析 DeviceInfo
fn parse_device_info(
    props: &btleplug::api::PeripheralProperties,
) -> Option<DeviceInfo> {
    // 策略 1: 从制造商数据中解析
    if let Some(ref mfr_data) = props.manufacturer_data {
        for (_company_id, data) in mfr_data.iter() {
            if data.len() >= 9 {
                // 尝试 bincode 反序列化
                if let Ok(info) = protocol::deserialize::<DeviceInfo>(data) {
                    return Some(info);
                }
            }
        }
    }

    // 策略 2: 从服务数据中解析
    if let Some(ref svc_data) = props.service_data {
        for (_uuid, data) in svc_data.iter() {
            if let Ok(info) = protocol::deserialize::<DeviceInfo>(data) {
                return Some(info);
            }
        }
    }

    None
}
