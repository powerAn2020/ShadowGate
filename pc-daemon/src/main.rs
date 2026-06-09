//! ShadowGate Windows PC 守护进程
//!
//! 负责 BLE 扫描、设备认证、状态管理、系统锁屏/解锁。
//! 通过异步事件循环驱动所有操作。

mod ble_scanner;
mod challenge;
mod device_store;
mod ipc;
mod lock_actions;
mod power_monitor;
mod state_machine;

use anyhow::Result;
use log::{info, warn};
use shadowgate_core::ShadowGateConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

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

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    info!("ShadowGate PC Daemon starting...");

    let data_dir = ipc::program_data_dir();
    let persisted_config_path = ipc::persisted_config_path(&data_dir);

    // 加载配置
    let config = if let Some(cfg) = ipc::load_persisted_config(&persisted_config_path) {
        info!("Loaded persisted config from {:?}", persisted_config_path);
        cfg
    } else {
        match ShadowGateConfig::from_file(&config_path()) {
            Ok(cfg) => {
                info!("Loaded config from {:?}", config_path());
                cfg
            }
            Err(e) => {
                warn!("Failed to load config: {} — using defaults", e);
                ShadowGateConfig::default_config()
            }
        }
    };

    let runtime = Arc::new(Mutex::new(ipc::DaemonRuntime::new(
        config,
        persisted_config_path,
        data_dir,
    )));

    info!("Entering IPC event loop on {}", ipc::PIPE_NAME);
    ipc::run_ipc_server(runtime).await
}
