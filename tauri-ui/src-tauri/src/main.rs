#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! ShadowGate Tauri configuration UI backend.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::{process::CommandChild, ShellExt};

const PIPE_NAME: &str = r"\\.\pipe\shadowgate-daemon-v1";
const CREDENTIAL_TARGET: &str = "ShadowGate:WindowsUnlock";

struct AppState {
    daemon_child: Option<CommandChild>,
}

impl Drop for AppState {
    fn drop(&mut self) {
        if let Some(child) = self.daemon_child.take() {
            let _ = child.kill();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStatus {
    pub state: String,
    pub rssi: Option<f64>,
    pub device_name: Option<String>,
    pub uptime_seconds: u64,
    pub daemon_available: bool,
    pub trusted_device_count: usize,
    pub credential_ready: bool,
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
    pub service_uuid: String,
    pub unlock_threshold: i32,
    pub lock_threshold: i32,
    pub scan_interval_ms: u64,
    pub challenge_timeout_ms: u64,
    pub lock_confirmation_ms: u64,
    pub unlock_method: String,
}

#[derive(Debug, Deserialize)]
struct IpcEnvelope {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DaemonDevice {
    device_hash: String,
    name: String,
    public_key_hex: String,
    paired_at: u64,
    last_auth_at: Option<u64>,
}

fn ipc_request(cmd: &str, payload: Value) -> Result<Value, String> {
    #[cfg(windows)]
    {
        let mut pipe = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(PIPE_NAME)
            .map_err(|e| format!("daemon unavailable: {e}"))?;
        let request = json!({ "cmd": cmd, "payload": payload });
        let mut request_text = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        request_text.push('\n');
        pipe.write_all(request_text.as_bytes())
            .map_err(|e| e.to_string())?;
        pipe.flush().map_err(|e| e.to_string())?;

        let mut response = String::new();
        let mut reader = BufReader::new(pipe);
        reader.read_line(&mut response).map_err(|e| e.to_string())?;
        let envelope: IpcEnvelope = serde_json::from_str(&response).map_err(|e| e.to_string())?;
        if envelope.ok {
            Ok(envelope.data.unwrap_or(Value::Null))
        } else {
            Err(envelope.error.unwrap_or_else(|| "daemon error".to_string()))
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (cmd, payload);
        Err("daemon IPC is only available on Windows".to_string())
    }
}

fn ensure_daemon(app: &AppHandle, state: &State<Mutex<AppState>>) -> Result<(), String> {
    if state.lock().unwrap().daemon_child.is_some() {
        return Ok(());
    }

    let child = match app.shell().sidecar("shadowgate-daemon") {
        Ok(command) => {
            let (_rx, child) = command.spawn().map_err(|e| e.to_string())?;
            child
        }
        Err(sidecar_error) => {
            let dev_path = app
                .path()
                .resolve(
                    "../../target/release/shadowgate-daemon.exe",
                    tauri::path::BaseDirectory::Resource,
                )
                .map_err(|e| format!("{sidecar_error}; dev path resolve failed: {e}"))?;
            let (_rx, child) = app
                .shell()
                .command(dev_path)
                .spawn()
                .map_err(|e| format!("{sidecar_error}; dev spawn failed: {e}"))?;
            child
        }
    };

    state.lock().unwrap().daemon_child = Some(child);
    std::thread::sleep(std::time::Duration::from_millis(250));
    Ok(())
}

#[tauri::command]
fn get_status(app: AppHandle, state: State<Mutex<AppState>>) -> AppStatus {
    let _ = ensure_daemon(&app, &state);
    match ipc_request("status", Value::Null) {
        Ok(value) => {
            let mut status: AppStatus =
                serde_json::from_value(value).unwrap_or_else(|_| offline_status());
            status.credential_ready = status.credential_ready && windows_credential_available();
            status
        }
        Err(_) => offline_status(),
    }
}

#[tauri::command]
fn get_devices(app: AppHandle, state: State<Mutex<AppState>>) -> Vec<DeviceInfo> {
    let _ = ensure_daemon(&app, &state);
    ipc_request("list_devices", Value::Null)
        .ok()
        .and_then(|value| serde_json::from_value::<Vec<DaemonDevice>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|device| DeviceInfo {
            name: device.name,
            hash: device.device_hash,
            paired_at: device.paired_at.to_string(),
            last_auth: device.last_auth_at.map(|ts| ts.to_string()),
            public_key: device.public_key_hex,
        })
        .collect()
}

#[tauri::command]
fn begin_pairing(app: AppHandle, state: State<Mutex<AppState>>) -> Result<String, String> {
    ensure_daemon(&app, &state)?;
    let payload = ipc_request("begin_pairing", Value::Null)?;
    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())
}

#[tauri::command]
fn pair_device(
    qr_content: String,
    app: AppHandle,
    state: State<Mutex<AppState>>,
) -> Result<String, String> {
    ensure_daemon(&app, &state)?;
    let payload: Value = serde_json::from_str(qr_content.trim())
        .map_err(|e| format!("Expected Android pairing JSON: {e}"))?;
    ipc_request("finish_pairing", payload)?;
    Ok("Device paired".to_string())
}

#[tauri::command]
fn unpair_device(
    hash: String,
    app: AppHandle,
    state: State<Mutex<AppState>>,
) -> Result<String, String> {
    ensure_daemon(&app, &state)?;
    ipc_request("unpair_device", json!({ "device_hash": hash }))?;
    Ok("Device unpaired".to_string())
}

#[tauri::command]
fn get_config(app: AppHandle, state: State<Mutex<AppState>>) -> DaemonConfig {
    let _ = ensure_daemon(&app, &state);
    ipc_request("get_config", Value::Null)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(default_config)
}

#[tauri::command]
fn update_config(
    config_json: String,
    app: AppHandle,
    state: State<Mutex<AppState>>,
) -> Result<String, String> {
    ensure_daemon(&app, &state)?;
    let payload: Value =
        serde_json::from_str(&config_json).map_err(|e| format!("Invalid config: {e}"))?;
    ipc_request("set_config", payload)?;
    Ok("Config updated".to_string())
}

#[tauri::command]
fn get_logs(app: AppHandle, state: State<Mutex<AppState>>) -> Vec<String> {
    let _ = ensure_daemon(&app, &state);
    ipc_request("logs_tail", Value::Null)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| vec!["daemon offline".to_string()])
}

#[tauri::command]
fn toggle_daemon(app: AppHandle, state: State<Mutex<AppState>>) -> Result<bool, String> {
    ensure_daemon(&app, &state)?;
    let status: AppStatus = ipc_request("status", Value::Null)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(offline_status);
    if status.state == "IDLE" {
        ipc_request("start_scan", Value::Null)?;
        Ok(true)
    } else {
        ipc_request("stop_scan", Value::Null)?;
        Ok(false)
    }
}

#[tauri::command]
fn credential_status(app: AppHandle, state: State<Mutex<AppState>>) -> Value {
    let _ = ensure_daemon(&app, &state);
    let daemon = ipc_request("credential_status", Value::Null)
        .unwrap_or_else(|_| json!({ "ready": false }));
    json!({
        "daemon_authorization": daemon,
        "windows_credential": windows_credential_available(),
    })
}

#[tauri::command]
fn windows_credential_status() -> Value {
    json!({
        "target": CREDENTIAL_TARGET,
        "available": windows_credential_available(),
    })
}

#[tauri::command]
fn save_windows_credential(
    username: String,
    domain: String,
    password: String,
) -> Result<String, String> {
    write_windows_credential(&username, &domain, &password)?;
    Ok("Windows credential saved".to_string())
}

fn offline_status() -> AppStatus {
    AppStatus {
        state: "OFFLINE".to_string(),
        rssi: None,
        device_name: None,
        uptime_seconds: 0,
        daemon_available: false,
        trusted_device_count: 0,
        credential_ready: false,
    }
}

fn default_config() -> DaemonConfig {
    DaemonConfig {
        service_uuid: "7f4d0001-7d6a-4f8f-9a7d-4f1f68b0f001".to_string(),
        unlock_threshold: -60,
        lock_threshold: -80,
        scan_interval_ms: 1000,
        challenge_timeout_ms: 1500,
        lock_confirmation_ms: 5000,
        unlock_method: "credential_provider".to_string(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Mutex::new(AppState { daemon_child: None }))
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_devices,
            begin_pairing,
            pair_device,
            unpair_device,
            get_config,
            update_config,
            get_logs,
            toggle_daemon,
            credential_status,
            windows_credential_status,
            save_windows_credential,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ShadowGate UI");
}

#[cfg(not(mobile))]
fn main() {
    run();
}

#[cfg(windows)]
fn to_wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn normalized_username(username: &str, domain: &str) -> String {
    let username = username.trim();
    let domain = domain.trim();
    if domain.is_empty() || username.contains('\\') || username.contains('@') {
        username.to_string()
    } else {
        format!("{domain}\\{username}")
    }
}

#[cfg(windows)]
fn windows_credential_available() -> bool {
    use windows_sys::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let target = to_wide(CREDENTIAL_TARGET);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } != 0;
    if ok && !credential.is_null() {
        unsafe { CredFree(credential.cast()) };
    }
    ok
}

#[cfg(not(windows))]
fn windows_credential_available() -> bool {
    false
}

#[cfg(windows)]
fn write_windows_credential(username: &str, domain: &str, password: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Security::Credentials::{
        CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    let account = normalized_username(username, domain);
    if account.trim().is_empty() {
        return Err("username is required".to_string());
    }
    if password.is_empty() {
        return Err("password is required".to_string());
    }

    let mut target = to_wide(CREDENTIAL_TARGET);
    let mut user = to_wide(&account);
    let mut comment = to_wide("ShadowGate unlock credential");
    let mut secret = to_wide(password);

    let mut credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        Comment: comment.as_mut_ptr(),
        LastWritten: FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        },
        CredentialBlobSize: (secret.len() * std::mem::size_of::<u16>()) as u32,
        CredentialBlob: secret.as_mut_ptr().cast(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: std::ptr::null_mut(),
        UserName: user.as_mut_ptr(),
    };

    let ok = unsafe { CredWriteW(&mut credential, 0) } != 0;
    if ok {
        Ok(())
    } else {
        Err(format!(
            "CredWriteW failed: {}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(not(windows))]
fn write_windows_credential(_username: &str, _domain: &str, _password: &str) -> Result<(), String> {
    Err("Windows Credential Manager is only available on Windows".to_string())
}
