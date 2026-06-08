//! 状态机模块
//!
//! IDLE -> SCANNING -> AUTHENTICATING -> UNLOCKED -> MONITORING

use anyhow::Result;
use log::{info, warn};
use shadowgate_core::rssi_filter::{HysteresisAction, HysteresisDetector, KalmanFilter};
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ble_scanner::{BleScanner, DiscoveredDevice};
use crate::challenge::ChallengeRunner;
use crate::device_store::DeviceStore;
use crate::lock_actions::{lock_workstation, unlock_workstation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemState {
    Idle,
    Scanning,
    Authenticating,
    Unlocked,
    Monitoring,
}

impl std::fmt::Display for SystemState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemState::Idle => write!(f, "IDLE"),
            SystemState::Scanning => write!(f, "SCANNING"),
            SystemState::Authenticating => write!(f, "AUTHENTICATING"),
            SystemState::Unlocked => write!(f, "UNLOCKED"),
            SystemState::Monitoring => write!(f, "MONITORING"),
        }
    }
}

pub struct AppContext {
    pub config: ShadowGateConfig,
    pub device_store: DeviceStore,
    pub current_rssi: Option<f64>,
    pub connected_device: Option<DiscoveredDevice>,
}

pub struct StateMachine {
    state: SystemState,
    config: ShadowGateConfig,
    ctx: Arc<Mutex<AppContext>>,
}

impl StateMachine {
    pub fn new(config: ShadowGateConfig, ctx: Arc<Mutex<AppContext>>) -> Self {
        StateMachine {
            state: SystemState::Idle,
            config,
            ctx,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("State machine starting...");

        loop {
            match self.state {
                SystemState::Idle => {
                    info!("State: IDLE -> SCANNING");
                    self.state = SystemState::Scanning;
                }
                SystemState::Scanning => self.run_scanning_loop().await?,
                SystemState::Authenticating => self.run_auth().await?,
                SystemState::Unlocked => {
                    info!("State: UNLOCKED -> MONITORING");
                    self.state = SystemState::Monitoring;
                }
                SystemState::Monitoring => self.run_monitoring_loop().await?,
            }
        }
    }

    async fn run_scanning_loop(&mut self) -> Result<()> {
        let trusted_hashes = self.trusted_hashes().await;
        if trusted_hashes.is_empty() {
            warn!("No trusted devices configured; staying in SCANNING");
        }

        let mut scanner = BleScanner::new(self.config.clone()).await?;
        let service_uuid = self.config.ble.service_uuid.parse()?;
        let mut selected_device: Option<DiscoveredDevice> = None;
        let mut filter = KalmanFilter::default();
        let mut detector = HysteresisDetector::new(
            self.config.rssi.unlock_threshold_dbm as f64,
            self.config.rssi.lock_threshold_dbm as f64,
            self.config.rssi.hysteresis_samples,
            self.config.rssi.hysteresis_samples,
        );

        info!("Scanning for trusted devices matching {}...", service_uuid);
        scanner
            .scan(&service_uuid, |device| {
                if !trusted_hashes.contains(&device.device_info.device_hash) {
                    return false;
                }

                let smoothed_rssi = filter.update(device.rssi as f64);
                if detector.update(smoothed_rssi) == HysteresisAction::Unlock {
                    info!(
                        "Trusted device reached unlock threshold: raw={}dBm filtered={:.1}dBm",
                        device.rssi, smoothed_rssi
                    );
                    selected_device = Some(device);
                    return true;
                }

                false
            })
            .await?;

        if let Some(device) = selected_device {
            let mut ctx = self.ctx.lock().await;
            ctx.current_rssi = Some(device.rssi as f64);
            ctx.connected_device = Some(device);
            self.state = SystemState::Authenticating;
        }

        Ok(())
    }

    async fn run_auth(&mut self) -> Result<()> {
        let device = {
            let mut ctx = self.ctx.lock().await;
            ctx.connected_device.take()
        };

        let Some(device) = device else {
            warn!("Authentication requested without a selected device");
            self.state = SystemState::Scanning;
            return Ok(());
        };

        let runner = ChallengeRunner::new(self.config.clone(), self.ctx.clone());
        if runner.run(device.clone()).await? {
            unlock_workstation().map_err(anyhow::Error::msg)?;
            let mut ctx = self.ctx.lock().await;
            ctx.connected_device = Some(device);
            info!("Authentication passed -> UNLOCKED");
            self.state = SystemState::Unlocked;
        } else {
            warn!("Authentication failed -> SCANNING");
            self.state = SystemState::Scanning;
        }

        Ok(())
    }

    async fn run_monitoring_loop(&mut self) -> Result<()> {
        let target_hash = {
            let ctx = self.ctx.lock().await;
            ctx.connected_device
                .as_ref()
                .map(|device| device.device_info.device_hash)
        };

        let Some(target_hash) = target_hash else {
            self.state = SystemState::Scanning;
            return Ok(());
        };

        let mut scanner = BleScanner::new(self.config.clone()).await?;
        let service_uuid = self.config.ble.service_uuid.parse()?;
        let lock_samples = lock_sample_count(&self.config);
        let mut filter = KalmanFilter::default();
        let mut detector = HysteresisDetector::new(
            self.config.rssi.unlock_threshold_dbm as f64,
            self.config.rssi.lock_threshold_dbm as f64,
            self.config.rssi.hysteresis_samples,
            lock_samples,
        );

        info!(
            "Monitoring trusted device; lock requires {} low RSSI samples",
            lock_samples
        );
        scanner
            .scan(&service_uuid, |device| {
                if device.device_info.device_hash != target_hash {
                    return false;
                }

                let smoothed_rssi = filter.update(device.rssi as f64);
                if detector.update(smoothed_rssi) == HysteresisAction::Lock {
                    info!(
                        "Trusted device reached lock threshold: raw={}dBm filtered={:.1}dBm",
                        device.rssi, smoothed_rssi
                    );
                    return true;
                }

                false
            })
            .await?;

        lock_workstation().map_err(anyhow::Error::msg)?;
        let mut ctx = self.ctx.lock().await;
        ctx.connected_device = None;
        ctx.current_rssi = None;
        self.state = SystemState::Scanning;
        Ok(())
    }

    async fn trusted_hashes(&self) -> Vec<[u8; 8]> {
        let ctx = self.ctx.lock().await;
        ctx.device_store
            .all_devices()
            .iter()
            .filter_map(|device| hex_hash_to_bytes(&device.device_hash))
            .collect()
    }
}

fn lock_sample_count(config: &ShadowGateConfig) -> usize {
    let interval = config.rssi.rssi_sample_interval_ms.max(1);
    let by_duration = config.rssi.lock_confirmation_ms.div_ceil(interval) as usize;
    by_duration.max(config.rssi.hysteresis_samples)
}

fn hex_hash_to_bytes(hex: &str) -> Option<[u8; 8]> {
    if hex.len() != 16 {
        return None;
    }

    let mut out = [0u8; 8];
    for (idx, slot) in out.iter_mut().enumerate() {
        let start = idx * 2;
        *slot = u8::from_str_radix(&hex[start..start + 2], 16).ok()?;
    }
    Some(out)
}
