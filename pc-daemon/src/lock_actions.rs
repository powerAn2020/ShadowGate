//! 系统锁屏/解锁动作模块 (Windows)
//!
//! 使用 windows-rs 调用 Win32 API

use log::{error, info};

/// 锁定 Windows 工作站
pub fn lock_workstation() -> Result<(), String> {
    info!("Locking workstation...");

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::LockWorkStation;
        LockWorkStation().map_err(|e| format!("LockWorkStation failed: {:?}", e))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        info!("[mock] Workstation locked");
    }

    info!("Workstation locked");
    Ok(())
}

/// 解锁 Windows 工作站 (框架代码)
pub fn unlock_workstation() -> Result<(), String> {
    info!("Unlocking workstation...");

    #[cfg(target_os = "windows")]
    {
        // SendInput 方式: 模拟 Win+L 然后输入密码
        // 完整实现需 Credential Provider — 此处为框架
        simulate_input();
    }

    #[cfg(not(target_os = "windows"))]
    {
        info!("[mock] Workstation unlocked");
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn simulate_input() {
    use std::mem;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_KEYUP, VK_LCONTROL, VK_LMENU, VK_RETURN,
    };

    unsafe {
        // Send Ctrl+Alt+Enter (placeholder for actual unlock sequence)
        keybd_event(VK_LCONTROL, 0, 0, 0);
        keybd_event(VK_LMENU, 0, 0, 0);
        keybd_event(VK_RETURN, 0, 0, 0);
        keybd_event(VK_RETURN, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_LMENU, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_LCONTROL, 0, KEYEVENTF_KEYUP, 0);
    }
}
