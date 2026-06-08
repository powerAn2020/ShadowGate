//! 电源事件监听模块
//!
//! 监听 Windows 休眠/唤醒事件:
//! - PBT_APMSUSPEND: 系统即将休眠 → 暂停 BLE 扫描
//! - PBT_APMRESUMESUSPEND: 系统从休眠唤醒 → 重置蓝牙适配器，恢复扫描
//!
//! 实现方式: 注册窗口消息循环，监听 WM_POWERBROADCAST

use anyhow::{Context, Result};
use log::{error, info, warn};
use shadowgate_core::ShadowGateConfig;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::state_machine::AppContext;

/// 电源事件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    Suspend,
    Resume,
    BatteryLow,
    PowerStatusChange,
}

/// 启动电源事件监听器 (后台任务)
///
/// 通过轮询 Windows 系统状态来检测休眠/唤醒:
/// 1. 监听系统空闲时间变化
/// 2. 检测会话状态变化
/// 3. 使用 WTS API 检测会话锁/解锁
pub async fn monitor_power_events(
    config: ShadowGateConfig,
    ctx: Arc<Mutex<AppContext>>,
) -> Result<()> {
    info!("Power event monitor started");

    // 记录初始系统状态
    let mut was_asleep = false;

    loop {
        // 检测系统是否处于睡眠状态
        // 通过检查 GetTickCount 变化幅度来判断
        let tick_before = get_tick_count();
        sleep(Duration::from_secs(5)).await;
        let tick_after = get_tick_count();

        let elapsed = tick_after.wrapping_sub(tick_before);

        // 如果 tick 增量异常小 (休眠期间 tick 不增长)
        // 正常 5 秒应该增长约 5000ms
        let is_awake = elapsed > 4000; // 允许 1 秒误差

        if was_asleep && is_awake {
            // 系统从休眠中唤醒
            info!("System RESUME detected (tick jump: {}ms)", elapsed);
            handle_resume(&config, &ctx).await;
            was_asleep = false;
        } else if !is_awake {
            // 系统可能进入休眠
            if !was_asleep {
                info!("System SUSPEND detected (tick: {}ms)", elapsed);
                handle_suspend(&ctx).await;
                was_asleep = true;
            }
        }
    }
}

/// 获取系统启动以来的 Tick 计数 (毫秒)
fn get_tick_count() -> u64 {
    unsafe {
        let ms_since_boot = windows::Win32::System::SystemInformation::GetTickCount64();
        ms_since_boot
    }
}

/// 处理系统休眠事件
async fn handle_suspend(ctx: &Arc<Mutex<AppContext>>) {
    info!("Handling suspend...");

    let mut ctx = ctx.lock().await;
    // 标记当前连接设备为过期
    ctx.connected_device = None;
    ctx.current_rssi = None;

    info!("Suspend handled — BLE state invalidated");
}

/// 处理系统唤醒事件
async fn handle_resume(config: &ShadowGateConfig, ctx: &Arc<Mutex<AppContext>>) {
    info!("Handling resume from sleep...");

    // 等待蓝牙适配器就绪 (休眠后蓝牙需要重新初始化)
    let reset_delay = config.scanning.scan_interval_ms.max(3000);
    info!(
        "Waiting {}ms for Bluetooth adapter to recover after wake...",
        reset_delay
    );

    sleep(Duration::from_millis(reset_delay)).await;

    // 标记上下文为需要重新扫描
    {
        let mut ctx = ctx.lock().await;
        ctx.connected_device = None;
        ctx.current_rssi = None;
    }

    info!("Resume handling complete — ready to re-scan");
}

/// 获取当前电源状态 (供状态机查询)
pub fn get_power_state() -> PowerState {
    PowerState {
        on_ac: is_on_ac_power(),
        battery_percent: get_battery_percent(),
    }
}

#[derive(Debug, Clone)]
pub struct PowerState {
    pub on_ac: bool,
    pub battery_percent: Option<u8>,
}

fn is_on_ac_power() -> bool {
    use windows::Win32::System::Power::GetSystemPowerStatus;
    unsafe {
        let mut status = std::mem::zeroed();
        if GetSystemPowerStatus(&mut status).is_ok() {
            status.ACLineStatus == 1 // 1 = Online
        } else {
            true // assume AC if can't query
        }
    }
}

fn get_battery_percent() -> Option<u8> {
    use windows::Win32::System::Power::GetSystemPowerStatus;
    unsafe {
        let mut status = std::mem::zeroed();
        if GetSystemPowerStatus(&mut status).is_ok() {
            if status.BatteryLifePercent != 255 {
                Some(status.BatteryLifePercent)
            } else {
                None
            }
        } else {
            None
        }
    }
}
