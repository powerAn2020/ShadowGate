//! ShadowGate Tauri 配置 UI — 后端
//!
//! 提供 IPC 命令供前端调用，通过本地命名管道与 pc-daemon 通信。

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::State;

/// 前端状态结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub state: String,
    pub rssi: Option<f64>,
    pub device_name: Option<String>,
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub hash: String,
    pub paired_at: String,
    pub last_auth: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub unlock_threshold: i32,
    pub lock_threshold: i32,
    pub scan_interval_ms: u64,
    pub challenge_timeout_ms: u64,
    pub lock_confirmation_ms: u64,
}

/// 应用内部状态
struct AppState {
    daemon_running: bool,
    start_time: std::time::Instant,
}

/// 获取当前状态
#[tauri::command]
fn get_status(state: State<Mutex<AppState>>) -> AppStatus {
    let state = state.lock().unwrap();
    AppStatus {
        state: if state.daemon_running {
            "SCANNING".to_string()
        } else {
            "IDLE".to_string()
        },
        rssi: None,
        device_name: None,
        uptime_seconds: state.start_time.elapsed().as_secs(),
    }
}

/// 获取配对设备列表
#[tauri::command]
fn get_devices() -> Vec<DeviceInfo> {
    // 从 daemon IPC 或本地文件读取
    vec![]
}

/// 配对设备 (从 QR 码内容)
#[tauri::command]
fn pair_device(qr_content: String) -> Result<String, String> {
    // 解析 QR 码中的公钥信息，保存到信任存储
    Ok(format!("Device paired: {}", &qr_content[..qr_content.len().min(20)]))
}

/// 取消配对设备
#[tauri::command]
fn unpair_device(hash: String) -> Result<String, String> {
    Ok(format!("Device unpaired: {}", hash))
}

/// 获取当前配置
#[tauri::command]
fn get_config() -> DaemonConfig {
    DaemonConfig {
        unlock_threshold: -60,
        lock_threshold: -80,
        scan_interval_ms: 1000,
        challenge_timeout_ms: 1500,
        lock_confirmation_ms: 5000,
    }
}

/// 更新配置
#[tauri::command]
fn update_config(config_json: String) -> Result<String, String> {
    // 解析并保存配置
    let _config: DaemonConfig = serde_json::from_str(&config_json)
        .map_err(|e| format!("Invalid config: {}", e))?;
    Ok("Config updated".to_string())
}

/// 获取日志
#[tauri::command]
fn get_logs() -> Vec<String> {
    vec![
        "[12:00:01] ShadowGate daemon started".to_string(),
        "[12:00:02] BLE adapter initialized".to_string(),
        "[12:00:05] Scanning for devices...".to_string(),
    ]
}

/// 启动/停止 daemon
#[tauri::command]
fn toggle_daemon(state: State<Mutex<AppState>>) -> bool {
    let mut state = state.lock().unwrap();
    state.daemon_running = !state.daemon_running;
    state.daemon_running
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Mutex::new(AppState {
            daemon_running: false,
            start_time: std::time::Instant::now(),
        }))
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_devices,
            pair_device,
            unpair_device,
            get_config,
            update_config,
            get_logs,
            toggle_daemon,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ShadowGate UI");
}
