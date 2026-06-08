//! 电源事件监听模块 (简化版)
//!
//! 通过轮询 GetTickCount 检测休眠/唤醒

use anyhow::Result;
use log::info;
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::state_machine::AppContext;

/// 启动电源事件监听器 (后台任务)
pub async fn monitor_power_events(
    _config: ShadowGateConfig,
    ctx: Arc<Mutex<AppContext>>,
) -> Result<()> {
    info!("Power event monitor started");

    let mut was_suspended = false;

    loop {
        let tick_before = get_tick_ms();
        sleep(Duration::from_secs(5)).await;
        let tick_after = get_tick_ms();
        let elapsed = tick_after.wrapping_sub(tick_before);

        if was_suspended && elapsed > 4000 {
            info!("System RESUME detected (tick={}ms)", elapsed);
            was_suspended = false;
            let mut ctx = ctx.lock().await;
            ctx.connected_device = None;
            ctx.current_rssi = None;
        } else if elapsed < 1000 && !was_suspended {
            info!("System SUSPEND detected (tick={}ms)", elapsed);
            was_suspended = true;
            let mut ctx = ctx.lock().await;
            ctx.connected_device = None;
            ctx.current_rssi = None;
        }
    }
}

fn get_tick_ms() -> u64 {
    #[cfg(target_os = "windows")]
    unsafe {
        windows::Win32::System::SystemInformation::GetTickCount64()
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::time::SystemTime;
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
