//! ShadowGate Windows PC 守护进程
//!
//! 负责 BLE 扫描、设备认证、状态管理、系统锁屏/解锁。
//! 通过异步事件循环驱动所有操作。

mod ble_scanner;
mod challenge;
mod device_store;
mod lock_actions;
mod power_monitor;
mod state_machine;

use anyhow::Result;
use log::{error, info, warn};
use shadowgate_core::ShadowGateConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use device_store::DeviceStore;
use state_machine::{AppContext, StateMachine};

/// 获取配置路径
fn config_path() -> PathBuf {
    // 优先使用同目录下的 config/default.toml
    let exe_dir = std::env::current_exe()
        .map(|p| p.parent().unwrap_or(PathBuf::new().as_ref()).to_path_buf())
        .unwrap_or_default();

    let candidate = exe_dir.join("config").join("default.toml");
    if candidate.exists() {
        return candidate;
    }

    // fallback: 项目源码目录
    PathBuf::from("../config/default.toml")
}

/// 获取应用数据目录
fn app_data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join("AppData").join("Roaming")
        });
    let dir = base.join("ShadowGate");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| {
        warn!("Failed to create app data dir: {}", e);
    });
    dir
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    info!("ShadowGate PC Daemon starting...");

    // 加载配置
    let config = match ShadowGateConfig::from_file(&config_path()) {
        Ok(cfg) => {
            info!("Loaded config from {:?}", config_path());
            cfg
        }
        Err(e) => {
            warn!("Failed to load config: {} — using defaults", e);
            ShadowGateConfig::default_config()
        }
    };

    // 加载设备存储
    let device_store = DeviceStore::load(&app_data_dir().join("trusted_devices.json"))
        .unwrap_or_else(|e| {
            warn!("Failed to load device store: {} — starting fresh", e);
            DeviceStore::new()
        });
    info!("Loaded {} trusted device(s)", device_store.count());

    // 创建共享应用上下文
    let ctx = Arc::new(Mutex::new(AppContext {
        config: config.clone(),
        device_store,
        current_rssi: None,
        connected_device: None,
    }));

    // 启动状态机
    let mut sm = StateMachine::new(config.clone(), ctx.clone());

    // 启动电源监听器 (后台任务)
    let power_ctx = ctx.clone();
    let power_config = config.clone();
    tokio::spawn(async move {
        if let Err(e) = power_monitor::monitor_power_events(power_config, power_ctx).await {
            error!("Power monitor failed: {}", e);
        }
    });

    // 运行主状态机事件循环
    info!("Entering main event loop...");
    if let Err(e) = sm.run().await {
        error!("State machine exited with error: {}", e);
        return Err(e);
    }

    info!("ShadowGate PC Daemon stopped.");
    Ok(())
}
