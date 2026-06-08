//! 系统锁屏/解锁动作模块
//!
//! 通过 windows-rs 调用 Win32 API 实现:
//! - LockWorkStation: 锁定工作站
//! - keybd_event / SendInput: 模拟键盘输入 (用于解锁)

use log::{error, info, warn};

/// 锁定 Windows 工作站
///
/// 调用 `LockWorkStation` API — 效果等同于 Win+L
pub fn lock_workstation() -> Result<(), String> {
    info!("Locking workstation...");

    unsafe {
        // LockWorkStation 是 user32.dll 导出的函数
        let result = windows::Win32::UI::Input::KeyboardAndMouse::LockWorkStation();
        if result.is_err() {
            let err = result.unwrap_err();
            error!("LockWorkStation failed: {:?}", err);
            return Err(format!("LockWorkStation failed: {:?}", err));
        }
    }

    info!("Workstation locked successfully");
    Ok(())
}

/// 解锁 Windows 工作站
///
/// 策略 1: 模拟按键序列 (Ctrl+Alt+Del 后输入密码)
/// 策略 2: 使用 Credential Provider (高级，需额外配置)
///
/// 注意: 模拟按键需要进程具有足够的权限，且 Windows 安全策略允许.
pub fn unlock_workstation() -> Result<(), String> {
    info!("Attempting to unlock workstation...");

    // 发送 Ctrl+Alt+Del 序列 (SAS - Secure Attention Sequence)
    // 这一步在 Vista+ 上需要特殊处理
    simulate_sas()?;

    // 短暂延迟等待安全桌面出现
    std::thread::sleep(std::time::Duration::from_millis(500));

    // 注意: 完整的自动解锁需要 Credential Provider 集成
    // 当前实现为框架代码，实际部署需要:
    // 1. 注册自定义 Credential Provider DLL
    // 2. 通过 IPC 传递解锁凭证
    //
    // 作为替代方案: 如果用户已设置 Windows Hello 或 PIN，
    // Ctrl+Alt+Del 可能已足够触发生物识别

    info!("SAS sequence sent — awaiting Windows authentication");
    Ok(())
}

/// 发送 Secure Attention Sequence (Ctrl+Alt+Del)
fn simulate_sas() -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::SystemServices::GENERIC_ALL;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_DELETE,
        VK_LCONTROL, VK_LMENU,
    };

    // 构造 Ctrl+Alt+Del 按键序列
    let inputs = [
        keybd_input(VK_LCONTROL, false),
        keybd_input(VK_LMENU, false),
        keybd_input(VK_DELETE, false),
        keybd_input(VK_DELETE, true),
        keybd_input(VK_LMENU, true),
        keybd_input(VK_LCONTROL, true),
    ];

    unsafe {
        let result = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if result == 0 {
            let err = windows::Win32::Foundation::GetLastError();
            error!("SendInput (SAS) failed: {:?}", err);
            return Err(format!("SendInput failed: {:?}", err));
        }
    }

    Ok(())
}

/// 构造单个键盘输入结构
fn keybd_input(vk: u16, key_up: bool) -> INPUT {
    let flags = if key_up { KEYEVENTF_KEYUP } else { 0 };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// 使用 SendSAS API (更可靠的方式)
///
/// 需要调用进程具有 SE_TCB_NAME 特权
/// 这在服务/守护进程中通常不可用，故作为备选方案
#[allow(dead_code)]
fn send_sas_via_api() -> Result<(), String> {
    // SendSAS 是 sas.dll 导出的，仅在特定场景可用
    // 保留此接口供后续 Credential Provider 集成使用
    warn!("SendSAS API not available — using SendInput fallback");
    Err("Not implemented — use Credential Provider instead".into())
}
