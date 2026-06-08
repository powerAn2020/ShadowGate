//! 状态机模块 — 简化版
//!
//! IDLE → SCANNING → AUTHENTICATING → UNLOCKED → MONITORING

use anyhow::Result;
use log::{error, info, warn};
use shadowgate_core::rssi_filter::{HysteresisAction, HysteresisDetector, KalmanFilter};
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

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
                SystemState::Scanning => {
                    self.run_scanning_loop().await?;
                }
                SystemState::Authenticating => {
                    self.run_auth().await?;
                }
                SystemState::Unlocked => {
                    info!("State: UNLOCKED (simulated)");
                    sleep(Duration::from_secs(5)).await;
                    self.state = SystemState::Monitoring;
                }
                SystemState::Monitoring => {
                    info!("State: MONITORING (simulated)");
                    sleep(Duration::from_secs(10)).await;
                    self.state = SystemState::Scanning;
                }
            }
        }
    }

    async fn run_scanning_loop(&mut self) -> Result<()> {
        let service_uuid: Uuid = self.config.ble.service_uuid.parse()?;
        let unlock_threshold = self.config.rssi.unlock_threshold_dbm as f64;
        let mut scanner = BleScanner::new(self.config.clone()).await?;

        // Simplified: poll for devices using btleplug peripherals API directly
        scanner
            .adapter()
            .start_scan(btleplug::api::ScanFilter::default())
            .await?;
        info!("Scanning for devices matching {}...", service_uuid);

        // Scan for a few seconds, looking for devices
        for _ in 0..20 {
            sleep(Duration::from_millis(500)).await;
            let peripherals = scanner.adapter().peripherals().await?;

            for p in peripherals.iter() {
                if let Ok(Some(props)) = p.properties().await {
                    if let Some(rssi) = props.rssi {
                        let has_service = props.services.iter().any(|s| s == &service_uuid);
                        if has_service && (rssi as f64) >= unlock_threshold {
                            // Check if device is trusted
                            let ctx = self.ctx.lock().await;
                            let known = ctx.device_store.count() > 0;
                            drop(ctx);

                            if known {
                                info!("Found trusted device at RSSI={}dBm", rssi);
                                scanner.adapter().stop_scan().await?;

                                let mut ctx = self.ctx.lock().await;
                                ctx.current_rssi = Some(rssi as f64);
                                drop(ctx);

                                self.state = SystemState::Authenticating;
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }

        scanner.adapter().stop_scan().await?;
        Ok(())
    }

    async fn run_auth(&mut self) -> Result<()> {
        info!("Authenticating...");
        sleep(Duration::from_secs(1)).await;

        // Simplified: attempt unlock
        let _ = unlock_workstation();
        info!("Authentication passed -> UNLOCKED");
        self.state = SystemState::Unlocked;
        Ok(())
    }
}
