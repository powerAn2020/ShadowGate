//! 状态机模块
//!
//! 维护 BLE 锁屏系统的生命周期状态流转:
//! IDLE → SCANNING → AUTHENTICATING → UNLOCKED → MONITORING
//!
//! 双向不对称迟滞: RSSI 连续 N 次高于解锁阈值 → 解锁
//!                   RSSI 连续 M 次低于锁定阈值 + 持续 T ms → 锁定

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
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

/// 系统状态枚举
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

/// 共享应用上下文
pub struct AppContext {
    pub config: ShadowGateConfig,
    pub device_store: DeviceStore,
    pub current_rssi: Option<f64>,
    pub connected_device: Option<DiscoveredDevice>,
}

/// 状态机
pub struct StateMachine {
    state: SystemState,
    config: ShadowGateConfig,
    ctx: Arc<Mutex<AppContext>>,
    filter: KalmanFilter,
    hysteresis: HysteresisDetector,
    /// 锁定确认计时: 记录首次低于锁定阈值的时间
    lock_start_time: Option<std::time::Instant>,
    /// 最近一次 RSSI 采样时间
    last_sample: std::time::Instant,
}

impl StateMachine {
    pub fn new(config: ShadowGateConfig, ctx: Arc<Mutex<AppContext>>) -> Self {
        let hysteresis = HysteresisDetector::new(
            config.rssi.unlock_threshold_dbm as f64,
            config.rssi.lock_threshold_dbm as f64,
            config.rssi.hysteresis_samples,
            config.rssi.hysteresis_samples + 2, // 锁定需要更多确认
        );

        StateMachine {
            state: SystemState::Idle,
            config,
            ctx,
            filter: KalmanFilter::default(),
            hysteresis,
            lock_start_time: None,
            last_sample: std::time::Instant::now(),
        }
    }

    /// 运行主事件循环
    pub async fn run(&mut self) -> Result<()> {
        info!("State machine starting...");

        loop {
            match self.state {
                SystemState::Idle => {
                    self.transition_to(SystemState::Scanning).await?;
                }
                SystemState::Scanning => {
                    self.run_scanning().await?;
                }
                SystemState::Authenticating => {
                    self.run_authenticating().await?;
                }
                SystemState::Unlocked => {
                    self.run_unlocked().await?;
                }
                SystemState::Monitoring => {
                    self.run_monitoring().await?;
                }
            }
        }
    }

    /// 状态转换
    async fn transition_to(&mut self, new_state: SystemState) -> Result<()> {
        info!("State transition: {} -> {}", self.state, new_state);
        self.state = new_state;

        match new_state {
            SystemState::Unlocked => {
                info!("Executing unlock action...");
                if let Err(e) = unlock_workstation() {
                    error!("Failed to unlock workstation: {}", e);
                    // 解锁失败，回退到扫描状态
                    self.transition_to(SystemState::Scanning).await?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// 扫描状态 — 持续扫描 BLE 设备
    async fn run_scanning(&mut self) -> Result<()> {
        info!("Entering SCANNING state...");

        let service_uuid: Uuid = self
            .config
            .ble
            .service_uuid
            .parse()
            .context("Invalid SERVICE_UUID in config")?;

        let mut scanner = BleScanner::new(self.config.clone()).await?;

        let ctx = self.ctx.clone();
        let mut self_ref = self as *mut StateMachine;

        let result = scanner
            .scan(&service_uuid, |device: DiscoveredDevice| {
                // 在回调中检查设备是否在信任列表中
                let ctx_guard = futures::executor::block_on(async { ctx.lock().await });
                let known = ctx_guard
                    .device_store
                    .get_by_hash(&device.device_info.device_hash);
                drop(ctx_guard);

                if known.is_none() {
                    debug!(
                        "Unknown device {:02x?}, skipping",
                        device.device_info.device_hash
                    );
                    return false; // 继续扫描
                }

                // 发现已配对设备，更新 RSSI
                let rssi = device.rssi as f64;

                // SAFETY: 在单线程回调中访问 self
                let sm = unsafe { &mut *self_ref };
                let filtered_rssi = sm.filter.update(rssi);

                debug!(
                    "Known device RSSI: raw={} dBm, filtered={:.1} dBm",
                    rssi, filtered_rssi
                );

                // 迟滞检测
                let action = sm.hysteresis.update(filtered_rssi);
                match action {
                    HysteresisAction::Unlock => {
                        info!("Hysteresis triggered UNLOCK at RSSI={:.1}", filtered_rssi);
                        // 设置连接的设备
                        let mut ctx = futures::executor::block_on(async { ctx.lock().await });
                        ctx.connected_device = Some(device);
                        ctx.current_rssi = Some(filtered_rssi);
                        drop(ctx);

                        // 停止扫描，进入认证状态
                        return true;
                    }
                    HysteresisAction::Lock => {
                        debug!("Hysteresis suggests LOCK (but we are in SCANNING state)");
                    }
                    HysteresisAction::None => {}
                }

                false // 继续扫描
            })
            .await;

        match result {
            Ok(()) => {
                // scan 返回是因为回调返回了 true → 触发解锁
                self.transition_to(SystemState::Authenticating).await?;
            }
            Err(e) => {
                error!("Scan error: {}", e);
                // 短暂延迟后重试
                sleep(Duration::from_secs(5)).await;
            }
        }

        Ok(())
    }

    /// 认证状态 — 执行 Challenge-Response
    async fn run_authenticating(&mut self) -> Result<()> {
        info!("Entering AUTHENTICATING state...");

        let device = {
            let ctx = self.ctx.lock().await;
            ctx.connected_device.clone()
        };

        let device = match device {
            Some(d) => d,
            None => {
                warn!("No device to authenticate — returning to SCANNING");
                self.transition_to(SystemState::Scanning).await?;
                return Ok(());
            }
        };

        // 执行 Challenge-Response 认证
        let challenge_runner = ChallengeRunner::new(self.config.clone(), self.ctx.clone());

        match challenge_runner.run(device).await {
            Ok(true) => {
                info!("Authentication SUCCESS — unlocking...");
                self.transition_to(SystemState::Unlocked).await?;
            }
            Ok(false) => {
                warn!("Authentication FAILED — signature mismatch or timeout");
                self.transition_to(SystemState::Scanning).await?;
            }
            Err(e) => {
                error!("Authentication error: {}", e);
                self.transition_to(SystemState::Scanning).await?;
            }
        }

        Ok(())
    }

    /// 已解锁状态 — 已认证后短暂确认
    async fn run_unlocked(&mut self) -> Result<()> {
        info!("Entering UNLOCKED state — device in range, PC unlocked");

        // 短暂停留后进入监测状态
        sleep(Duration::from_secs(2)).await;

        self.hysteresis.reset();
        self.lock_start_time = None;
        self.transition_to(SystemState::Monitoring).await?;

        Ok(())
    }

    /// 监测状态 — 持续监控 RSSI，低于阈值则锁定
    async fn run_monitoring(&mut self) -> Result<()> {
        info!("Entering MONITORING state — watching for device departure");

        let service_uuid: Uuid = self
            .config
            .ble
            .service_uuid
            .parse()
            .context("Invalid SERVICE_UUID in config")?;

        let mut scanner = BleScanner::new(self.config.clone()).await?;

        let ctx = self.ctx.clone();
        let connected_hash = {
            let ctx = self.ctx.lock().await;
            ctx.connected_device
                .as_ref()
                .map(|d| d.device_info.device_hash)
        };

        let target_hash = match connected_hash {
            Some(h) => h,
            None => {
                warn!("No connected device — returning to SCANNING");
                self.transition_to(SystemState::Scanning).await?;
                return Ok(());
            }
        };

        let mut self_ref = self as *mut StateMachine;
        let lock_threshold = self.config.rssi.lock_threshold_dbm as f64;
        let lock_confirmation_ms = self.config.rssi.lock_confirmation_ms;
        let sample_interval = self.config.rssi.rssi_sample_interval_ms;

        let result = scanner
            .scan(&service_uuid, |device: DiscoveredDevice| {
                if device.device_info.device_hash != target_hash {
                    return false; // 不是我们要监控的设备
                }

                let sm = unsafe { &mut *self_ref };
                let rssi = device.rssi as f64;
                let filtered_rssi = sm.filter.update(rssi);
                sm.last_sample = std::time::Instant::now();

                if filtered_rssi <= lock_threshold {
                    // RSSI 低于锁定阈值
                    if sm.lock_start_time.is_none() {
                        sm.lock_start_time = Some(std::time::Instant::now());
                        debug!(
                            "RSSI {:.1} <= {} (lock threshold) — starting lock timer",
                            filtered_rssi, lock_threshold
                        );
                    }

                    // 检查是否已持续足够长时间
                    let elapsed = sm.lock_start_time.unwrap().elapsed();
                    if elapsed.as_millis() as u64 >= lock_confirmation_ms {
                        // 还需要迟滞检测确认
                        let action = sm.hysteresis.update(filtered_rssi);
                        if action == HysteresisAction::Lock {
                            info!(
                                "LOCK triggered: RSSI {:.1} below threshold for {:?}",
                                filtered_rssi, elapsed
                            );
                            // 执行锁定
                            if let Err(e) = lock_workstation() {
                                error!("Failed to lock workstation: {}", e);
                            }
                            sm.hysteresis.reset();
                            sm.lock_start_time = None;
                            return true; // 停止扫描，返回 SCANNING
                        }
                    }
                } else {
                    // RSSI 回升到阈值以上
                    if sm.lock_start_time.is_some() {
                        debug!("RSSI recovered above lock threshold — resetting timer");
                        sm.lock_start_time = None;
                    }
                }

                false
            })
            .await;

        match result {
            Ok(()) => {
                // 设备离开，返回扫描状态
                {
                    let mut ctx = self.ctx.lock().await;
                    ctx.connected_device = None;
                    ctx.current_rssi = None;
                }
                self.transition_to(SystemState::Scanning).await?;
            }
            Err(e) => {
                error!("Monitor scan error: {}", e);
                sleep(Duration::from_secs(5)).await;
            }
        }

        Ok(())
    }
}
