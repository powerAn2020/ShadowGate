//! Local JSON-line IPC for the Tauri configuration UI.

use anyhow::{Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shadowgate_core::{KeyPair, ShadowGateConfig, PROTOCOL_VERSION};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::device_store::DeviceStore;
use crate::state_machine::{AppContext, StateMachine};

pub const PIPE_NAME: &str = r"\\.\pipe\shadowgate-daemon-v1";

pub struct DaemonRuntime {
    pub config: ShadowGateConfig,
    pub config_path: PathBuf,
    pub device_store: DeviceStore,
    pub device_store_path: PathBuf,
    pub log_path: PathBuf,
    pub started_at: Instant,
    pub scan_task: Option<JoinHandle<()>>,
    pub scan_ctx: Option<Arc<Mutex<AppContext>>>,
    pub logs: Vec<String>,
}

impl DaemonRuntime {
    pub fn new(config: ShadowGateConfig, config_path: PathBuf, data_dir: PathBuf) -> Self {
        let device_store_path = data_dir.join("trusted_devices.json");
        let device_store =
            DeviceStore::load(&device_store_path).unwrap_or_else(|_| DeviceStore::new());
        let log_path = data_dir.join("logs").join("daemon.log");
        let mut runtime = DaemonRuntime {
            config,
            config_path,
            device_store,
            device_store_path,
            log_path,
            started_at: Instant::now(),
            scan_task: None,
            scan_ctx: None,
            logs: Vec::new(),
        };
        runtime.push_log("daemon ipc initialized");
        runtime
    }

    fn push_log(&mut self, message: impl AsRef<str>) {
        let line = format!("[{}] {}", unix_seconds(), message.as_ref());
        self.logs.push(line.clone());
        if self.logs.len() > 500 {
            let overflow = self.logs.len() - 500;
            self.logs.drain(0..overflow);
        }

        if let Some(parent) = self.log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        use std::io::Write as _;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

#[derive(Debug, Deserialize)]
struct IpcRequest {
    cmd: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Serialize)]
struct IpcResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    pub service_uuid: String,
    pub unlock_threshold: i32,
    pub lock_threshold: i32,
    pub scan_interval_ms: u64,
    pub challenge_timeout_ms: u64,
    pub lock_confirmation_ms: u64,
    pub unlock_method: String,
}

#[derive(Debug, Serialize)]
struct StatusDto {
    state: String,
    daemon_available: bool,
    uptime_seconds: u64,
    rssi: Option<f64>,
    device_name: Option<String>,
    trusted_device_count: usize,
    credential_ready: bool,
}

#[derive(Debug, Serialize)]
struct TrustedDeviceDto {
    device_hash: String,
    name: String,
    public_key_hex: String,
    paired_at: u64,
    last_auth_at: Option<u64>,
    capabilities: u8,
}

#[derive(Debug, Deserialize)]
struct FinishPairingPayload {
    device_hash: String,
    public_key_hex: String,
    name: Option<String>,
    pairing_nonce: Option<String>,
}

pub fn program_data_dir() -> PathBuf {
    let base = std::env::var("PROGRAMDATA")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from("."));
    let dir = base.join("ShadowGate");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn persisted_config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("daemon-config.json")
}

pub fn load_persisted_config(path: &Path) -> Option<ShadowGateConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub async fn run_ipc_server(runtime: Arc<Mutex<DaemonRuntime>>) -> Result<()> {
    run_platform_ipc_server(runtime).await
}

#[cfg(windows)]
async fn run_platform_ipc_server(runtime: Arc<Mutex<DaemonRuntime>>) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

    loop {
        let pipe = ServerOptions::new()
            .pipe_mode(PipeMode::Message)
            .create(PIPE_NAME)
            .context("failed to create ShadowGate named pipe")?;
        pipe.connect().await.context("named pipe connect failed")?;
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(pipe);
            let mut lines = BufReader::new(reader).lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let response = match serde_json::from_str::<IpcRequest>(&line) {
                    Ok(request) => handle_request(runtime.clone(), request).await,
                    Err(e) => Err(anyhow::anyhow!("invalid request json: {e}")),
                };
                let response = match response {
                    Ok(data) => IpcResponse {
                        ok: true,
                        data: Some(data),
                        error: None,
                    },
                    Err(e) => IpcResponse {
                        ok: false,
                        data: None,
                        error: Some(e.to_string()),
                    },
                };
                if let Ok(mut text) = serde_json::to_string(&response) {
                    text.push('\n');
                    if writer.write_all(text.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
        });
    }
}

#[cfg(not(windows))]
async fn run_platform_ipc_server(runtime: Arc<Mutex<DaemonRuntime>>) -> Result<()> {
    runtime
        .lock()
        .await
        .push_log("named pipe ipc is only available on Windows");
    futures::future::pending::<()>().await;
    Ok(())
}

async fn handle_request(runtime: Arc<Mutex<DaemonRuntime>>, request: IpcRequest) -> Result<Value> {
    match request.cmd.as_str() {
        "status" => status(runtime).await,
        "get_config" => {
            let runtime = runtime.lock().await;
            Ok(json!(config_to_ui(&runtime.config)))
        }
        "set_config" => {
            let config: UiConfig = serde_json::from_value(request.payload)?;
            set_config(runtime, config).await
        }
        "list_devices" => {
            let runtime = runtime.lock().await;
            Ok(json!(list_devices(&runtime.device_store)))
        }
        "begin_pairing" => begin_pairing(runtime).await,
        "finish_pairing" => {
            let payload: FinishPairingPayload = serde_json::from_value(request.payload)?;
            finish_pairing(runtime, payload).await
        }
        "unpair_device" => {
            let device_hash = request
                .payload
                .get("device_hash")
                .and_then(Value::as_str)
                .context("device_hash is required")?
                .to_string();
            unpair_device(runtime, device_hash).await
        }
        "start_scan" => start_scan(runtime).await,
        "stop_scan" => stop_scan(runtime).await,
        "logs_tail" => {
            let runtime = runtime.lock().await;
            Ok(json!(runtime.logs))
        }
        "credential_status" => {
            let runtime = runtime.lock().await;
            let auth_file = credential_status_path(&runtime.log_path);
            Ok(json!({
                "ready": credential_authorization_ready(&auth_file),
                "auth_file": auth_file,
            }))
        }
        other => Err(anyhow::anyhow!("unknown command: {other}")),
    }
}

async fn status(runtime: Arc<Mutex<DaemonRuntime>>) -> Result<Value> {
    let (ctx, state, uptime_seconds, trusted_device_count, credential_ready) = {
        let mut runtime = runtime.lock().await;
        if runtime
            .scan_task
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
        {
            runtime.scan_task = None;
            runtime.scan_ctx = None;
            runtime.push_log("scan task finished");
        }

        (
            runtime.scan_ctx.clone(),
            if runtime.scan_task.is_some() {
                "SCANNING"
            } else {
                "IDLE"
            }
            .to_string(),
            runtime.started_at.elapsed().as_secs(),
            runtime.device_store.count(),
            credential_authorization_ready(&credential_status_path(&runtime.log_path)),
        )
    };

    let (rssi, device_name) = if let Some(ctx) = ctx {
        let ctx = ctx.lock().await;
        (
            ctx.current_rssi,
            ctx.connected_device.as_ref().and_then(|d| d.name.clone()),
        )
    } else {
        (None, None)
    };

    Ok(json!(StatusDto {
        state,
        daemon_available: true,
        uptime_seconds,
        rssi,
        device_name,
        trusted_device_count,
        credential_ready,
    }))
}

async fn set_config(runtime: Arc<Mutex<DaemonRuntime>>, ui: UiConfig) -> Result<Value> {
    if ui.unlock_threshold <= ui.lock_threshold {
        anyhow::bail!("unlock threshold must be greater than lock threshold");
    }
    let mut runtime = runtime.lock().await;
    runtime.config.ble.service_uuid = ui.service_uuid;
    runtime.config.rssi.unlock_threshold_dbm = ui.unlock_threshold;
    runtime.config.rssi.lock_threshold_dbm = ui.lock_threshold;
    runtime.config.scanning.scan_interval_ms = ui.scan_interval_ms;
    runtime.config.challenge.timeout_ms = ui.challenge_timeout_ms;
    runtime.config.rssi.lock_confirmation_ms = ui.lock_confirmation_ms;
    let content = serde_json::to_string_pretty(&runtime.config)?;
    std::fs::write(&runtime.config_path, content).context("failed to persist daemon config")?;
    runtime.push_log("configuration updated");
    Ok(json!(config_to_ui(&runtime.config)))
}

async fn begin_pairing(runtime: Arc<Mutex<DaemonRuntime>>) -> Result<Value> {
    let runtime = runtime.lock().await;
    let keypair = KeyPair::generate()?;
    let mut nonce = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce);
    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "pc_public_key_hex": hex::encode(keypair.public_key_bytes()),
        "pairing_nonce": hex::encode(nonce),
        "pipe": PIPE_NAME,
        "service_uuid": runtime.config.ble.service_uuid,
    }))
}

async fn finish_pairing(
    runtime: Arc<Mutex<DaemonRuntime>>,
    payload: FinishPairingPayload,
) -> Result<Value> {
    let mut runtime = runtime.lock().await;
    let name = payload
        .name
        .unwrap_or_else(|| "Android Credential".to_string());
    match runtime
        .device_store
        .add_device_hex(&payload.device_hash, name, &payload.public_key_hex)
    {
        Ok(()) => {}
        Err(e) if e.to_string().contains("already exists") => {
            runtime
                .device_store
                .remove_device_hex(&payload.device_hash)
                .ok();
            runtime.device_store.add_device_hex(
                &payload.device_hash,
                "Android Credential".to_string(),
                &payload.public_key_hex,
            )?;
        }
        Err(e) => return Err(e),
    }
    runtime.device_store.save(&runtime.device_store_path)?;
    runtime.push_log(format!(
        "paired device {}{}",
        payload.device_hash,
        payload
            .pairing_nonce
            .map(|nonce| format!(" via nonce {nonce}"))
            .unwrap_or_default()
    ));
    Ok(json!(list_devices(&runtime.device_store)))
}

async fn unpair_device(runtime: Arc<Mutex<DaemonRuntime>>, device_hash: String) -> Result<Value> {
    let mut runtime = runtime.lock().await;
    runtime.device_store.remove_device_hex(&device_hash)?;
    runtime.device_store.save(&runtime.device_store_path)?;
    runtime.push_log(format!("unpaired device {device_hash}"));
    Ok(json!(list_devices(&runtime.device_store)))
}

async fn start_scan(runtime: Arc<Mutex<DaemonRuntime>>) -> Result<Value> {
    let mut runtime = runtime.lock().await;
    if runtime.scan_task.is_some() {
        return Ok(json!({ "running": true }));
    }

    let ctx = Arc::new(Mutex::new(AppContext {
        config: runtime.config.clone(),
        device_store: runtime.device_store.clone(),
        current_rssi: None,
        connected_device: None,
    }));
    let mut sm = StateMachine::new(runtime.config.clone(), ctx.clone());
    let task = tokio::spawn(async move {
        if let Err(e) = sm.run().await {
            log::error!("state machine exited: {e}");
        }
    });

    runtime.scan_ctx = Some(ctx);
    runtime.scan_task = Some(task);
    runtime.push_log("scan task started");
    Ok(json!({ "running": true }))
}

async fn stop_scan(runtime: Arc<Mutex<DaemonRuntime>>) -> Result<Value> {
    let mut runtime = runtime.lock().await;
    if let Some(task) = runtime.scan_task.take() {
        task.abort();
    }
    runtime.scan_ctx = None;
    runtime.push_log("scan task stopped");
    Ok(json!({ "running": false }))
}

fn list_devices(store: &DeviceStore) -> Vec<TrustedDeviceDto> {
    store
        .all_devices()
        .iter()
        .map(|device| TrustedDeviceDto {
            device_hash: device.device_hash.clone(),
            name: device.name.clone(),
            public_key_hex: device.public_key_hex.clone(),
            paired_at: device.paired_at,
            last_auth_at: device.last_auth_at,
            capabilities: 0,
        })
        .collect()
}

fn config_to_ui(config: &ShadowGateConfig) -> UiConfig {
    UiConfig {
        service_uuid: config.ble.service_uuid.clone(),
        unlock_threshold: config.rssi.unlock_threshold_dbm,
        lock_threshold: config.rssi.lock_threshold_dbm,
        scan_interval_ms: config.scanning.scan_interval_ms,
        challenge_timeout_ms: config.challenge.timeout_ms,
        lock_confirmation_ms: config.rssi.lock_confirmation_ms,
        unlock_method: "credential_provider".to_string(),
    }
}

fn credential_status_path(log_path: &Path) -> PathBuf {
    log_path
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."))
        .join("credential_auth.json")
}

fn credential_authorization_ready(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return false;
    };
    let Some(authorized_until_ms) = value.get("authorized_until_ms").and_then(Value::as_u64) else {
        return false;
    };
    authorized_until_ms > unix_millis()
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
