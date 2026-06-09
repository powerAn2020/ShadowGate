#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! ShadowGate Tauri 配置 UI — 后端
//!
//! 提供 IPC 命令供前端调用，通过本地命名管道与 pc-daemon 通信。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// 前端状态结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub state: String,
    pub rssi: Option<f64>,
    pub device_name: Option<String>,
    pub uptime_seconds: u64,
    pub daemon_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub hash: String,
    pub paired_at: String,
    pub last_auth: Option<String>,
    pub public_key: String,
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
    data_dir: PathBuf,
    devices: Vec<DeviceInfo>,
    config: DaemonConfig,
    logs: Vec<String>,
}

impl AppState {
    fn new() -> Self {
        let data_dir = app_data_dir();
        let config = load_json(&data_dir.join("ui-config.json")).unwrap_or_else(default_config);
        let devices = load_json(&data_dir.join("trusted-devices-ui.json")).unwrap_or_default();
        let mut state = AppState {
            daemon_running: false,
            start_time: std::time::Instant::now(),
            data_dir,
            devices,
            config,
            logs: Vec::new(),
        };
        state.push_log("UI initialized");
        state.push_log("pc-daemon IPC is not connected yet");
        state
    }

    fn push_log(&mut self, message: impl AsRef<str>) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.logs.push(format!("[{}] {}", secs, message.as_ref()));
        if self.logs.len() > 200 {
            let overflow = self.logs.len() - 200;
            self.logs.drain(0..overflow);
        }
    }

    fn save_config(&self) -> Result<(), String> {
        save_json(&self.data_dir.join("ui-config.json"), &self.config)
    }

    fn save_devices(&self) -> Result<(), String> {
        save_json(
            &self.data_dir.join("trusted-devices-ui.json"),
            &self.devices,
        )
    }
}

fn app_data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("XDG_CONFIG_HOME").map(PathBuf::from))
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|home| PathBuf::from(home).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("."))
        });
    let dir = base.join("ShadowGate");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Option<T> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_json<T: Serialize>(path: &std::path::Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())
}

fn default_config() -> DaemonConfig {
    DaemonConfig {
        unlock_threshold: -60,
        lock_threshold: -80,
        scan_interval_ms: 1000,
        challenge_timeout_ms: 1500,
        lock_confirmation_ms: 5000,
    }
}

fn unix_time_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

fn derive_short_hash(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn parse_device(qr_content: &str) -> DeviceInfo {
    #[derive(Deserialize)]
    struct PairPayload {
        name: Option<String>,
        device_name: Option<String>,
        hash: Option<String>,
        device_hash: Option<String>,
        public_key: Option<String>,
        public_key_hex: Option<String>,
    }

    if let Ok(payload) = serde_json::from_str::<PairPayload>(qr_content) {
        let public_key = payload
            .public_key
            .or(payload.public_key_hex)
            .unwrap_or_else(|| qr_content.to_string());
        let hash = payload
            .hash
            .or(payload.device_hash)
            .unwrap_or_else(|| derive_short_hash(&public_key));
        return DeviceInfo {
            name: payload
                .name
                .or(payload.device_name)
                .unwrap_or_else(|| "Android Credential".to_string()),
            hash,
            paired_at: unix_time_string(),
            last_auth: None,
            public_key,
        };
    }

    DeviceInfo {
        name: "Android Credential".to_string(),
        hash: derive_short_hash(qr_content),
        paired_at: unix_time_string(),
        last_auth: None,
        public_key: qr_content.to_string(),
    }
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
        daemon_available: false,
    }
}

/// 获取配对设备列表
#[tauri::command]
fn get_devices(state: State<Mutex<AppState>>) -> Vec<DeviceInfo> {
    state.lock().unwrap().devices.clone()
}

/// 配对设备 (从 QR 码内容)
#[tauri::command]
fn pair_device(qr_content: String, state: State<Mutex<AppState>>) -> Result<String, String> {
    if qr_content.trim().is_empty() {
        return Err("QR content is empty".to_string());
    }

    let mut state = state.lock().unwrap();
    let device = parse_device(qr_content.trim());
    if let Some(existing) = state.devices.iter_mut().find(|d| d.hash == device.hash) {
        existing.name = device.name;
        existing.public_key = device.public_key;
        existing.paired_at = device.paired_at;
        let hash = existing.hash.clone();
        state.push_log(format!("Updated paired device {}", hash));
        state.save_devices()?;
        return Ok(format!("Device updated: {}", hash));
    }

    let hash = device.hash.clone();
    state.devices.push(device);
    state.push_log(format!("Paired device {}", hash));
    state.save_devices()?;
    Ok(format!("Device paired: {}", hash))
}

/// 取消配对设备
#[tauri::command]
fn unpair_device(hash: String, state: State<Mutex<AppState>>) -> Result<String, String> {
    let mut state = state.lock().unwrap();
    let before = state.devices.len();
    state.devices.retain(|device| device.hash != hash);
    if state.devices.len() == before {
        return Err(format!("Device not found: {}", hash));
    }
    state.push_log(format!("Unpaired device {}", hash));
    state.save_devices()?;
    Ok(format!("Device unpaired: {}", hash))
}

/// 获取当前配置
#[tauri::command]
fn get_config(state: State<Mutex<AppState>>) -> DaemonConfig {
    state.lock().unwrap().config.clone()
}

/// 更新配置
#[tauri::command]
fn update_config(config_json: String, state: State<Mutex<AppState>>) -> Result<String, String> {
    let config: DaemonConfig =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config: {}", e))?;
    if config.unlock_threshold <= config.lock_threshold {
        return Err("Unlock threshold must be greater than lock threshold".to_string());
    }
    if config.scan_interval_ms == 0 || config.challenge_timeout_ms == 0 {
        return Err("Intervals and timeouts must be greater than zero".to_string());
    }

    let mut state = state.lock().unwrap();
    state.config = config;
    state.save_config()?;
    state.push_log("Configuration saved");
    Ok("Config updated".to_string())
}

/// 获取日志
#[tauri::command]
fn get_logs(state: State<Mutex<AppState>>) -> Vec<String> {
    state.lock().unwrap().logs.clone()
}

/// 启动/停止 daemon
#[tauri::command]
fn toggle_daemon(state: State<Mutex<AppState>>) -> bool {
    let mut state = state.lock().unwrap();
    state.daemon_running = !state.daemon_running;
    if state.daemon_running {
        state.push_log("UI monitoring enabled; daemon IPC pending");
    } else {
        state.push_log("UI monitoring stopped");
    }
    state.daemon_running
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Mutex::new(AppState::new()))
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

#[cfg(not(mobile))]
fn main() {
    run();
}
