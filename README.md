# ShadowGate — BLE 跨端自动锁屏/解锁系统

> 基于 BLE 近场感知 + Ed25519 Challenge-Response 的跨平台自动锁屏系统

[![Rust](https://img.shields.io/badge/core-Rust-orange)](core-lib/)
[![Android](https://img.shields.io/badge/android-Kotlin-green)](android-app/)
[![Windows](https://img.shields.io/badge/windows-Tauri-blue)](tauri-ui/)
[![License](https://img.shields.io/badge/license-MIT-purple)](LICENSE)

---

## 项目架构

```
ShadowGate/
├── core-lib/              # Rust Core — 平台无关核心逻辑
│   ├── src/
│   │   ├── crypto.rs      # Ed25519 密钥生成/签名/验证
│   │   ├── rssi_filter.rs # 卡尔曼滤波 + 迟滞防抖
│   │   ├── protocol.rs    # Bincode 序列化协议
│   │   ├── config.rs      # TOML 配置管理
│   │   ├── ffi.rs         # Android JNI 接口
│   │   └── lib.rs         # 库入口
│   └── include/
│       └── shadowgate_core.h  # C FFI 头文件
│
├── pc-daemon/             # Windows PC 守护进程
│   └── src/
│       ├── main.rs        # 入口 + 事件循环
│       ├── ble_scanner.rs # BLE 扫描器 (btleplug)
│       ├── state_machine.rs # 状态机 (IDLE→SCANNING→AUTH→UNLOCKED→MONITOR)
│       ├── challenge.rs   # Challenge-Response 认证
│       ├── lock_actions.rs # Win32 锁屏/解锁 API
│       ├── device_store.rs # 信任设备 JSON 存储
│       └── power_monitor.rs # 休眠/唤醒检测
│
├── tauri-ui/              # Tauri 桌面配置 UI
│   ├── src-tauri/
│   │   └── src/main.rs    # Tauri 后端 + IPC
│   └── src/
│       ├── index.html     # 主界面壳
│       ├── styles.css     # 暗色玻璃态主题
│       └── main.js        # 前端逻辑
│
├── android-app/           # Android 凭证端
│   ├── app/src/main/
│   │   ├── AndroidManifest.xml
│   │   └── kotlin/com/shadowgate/app/
│   │       ├── ShadowGateApp.kt         # Application + 通知渠道
│   │       ├── crypto/NativeCrypto.kt   # JNI 桥接 + Keystore
│   │       ├── service/ShadowGateService.kt  # BLE 前台服务
│   │       └── ui/MainActivity.kt       # Compose 配置 UI
│   └── xposed/src/main/
│       ├── AndroidManifest.xml          # Xposed 模块声明
│       └── java/com/shadowgate/xposed/
│           └── ShadowGateXposedModule.kt # Doze 绕过 + BLE 保活
│
└── config/
    └── default.toml        # 全局参数配置
```

## 核心协议

### 1. 绑定配对 (Provisioning)

```
Android                          PC
  |                               |
  | <---- QR Code (PC 公钥) ----- |
  |                               |
  | ---- 公钥交换 (GATT) -------> |
  |                               |
  | <---- 签名验证确认 ---------- |
  |                               |
  ✓ 信任关系建立
```

### 2. 日常认证 (Challenge-Response)

```
Android                          PC
  |                               |
  | == BLE Advertise (Hash) ====> |  (持续扫描)
  |                               |  RSSI > -60dBm → 触发
  | <== Challenge (32B随机数) == |  (GATT Write)
  |                               |
  | == Response (Ed25519签名) ==> |  (GATT Notify)
  |                               |  验签通过 → 解锁
```

## 快速开始

### 前置条件

- **Windows**: Rust toolchain, Bluetooth 4.0+ 适配器
- **Android**: API 26+, BLE 硬件支持

### 编译 Core 库

```bash
cd ShadowGate

# 编译 + 测试 (host)
cargo build --release
cargo test

# Android 交叉编译 (.so)
cargo build --release --target aarch64-linux-android -p shadowgate-core
```

### 运行 PC 守护进程

```bash
cargo run --release -p shadowgate-pc-daemon
```

### 运行 Tauri 配置 UI

```bash
cd tauri-ui
npm install
npm run tauri dev
```

### 本地构建 Windows 桌面端

Tauri 打包时需要 `pc-daemon` 作为 sidecar。先生成并复制 sidecar，再构建桌面安装包：

```powershell
cargo build --release -p shadowgate-pc-daemon
New-Item -ItemType Directory -Force tauri-ui/src-tauri/bin | Out-Null
Copy-Item target/release/shadowgate-daemon.exe tauri-ui/src-tauri/bin/shadowgate-daemon-x86_64-pc-windows-msvc.exe -Force
cd tauri-ui
npm run build -- --no-sign
```

Tauri 会通过 named pipe `\\.\pipe\shadowgate-daemon-v1` 与 sidecar 通信。桌面 UI 中的 Credential Provider 设置会把 Windows 解锁凭据写入 Windows Credential Manager，target 固定为 `ShadowGate:WindowsUnlock`，不会写入普通 JSON 配置文件。

### 构建/安装 Credential Provider

Credential Provider 是 x64 C++ COM DLL，仅建议在 Windows 10/11 测试机上以管理员权限注册：

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\amd64\MSBuild.exe" credential-provider/ShadowGateCredentialProvider.vcxproj /p:Configuration=Release /p:Platform=x64
powershell -ExecutionPolicy Bypass -File credential-provider/register.ps1 -DllPath credential-provider/x64/Release/ShadowGateCredentialProvider.dll
```

卸载：

```powershell
powershell -ExecutionPolicy Bypass -File credential-provider/unregister.ps1 -DllPath credential-provider/x64/Release/ShadowGateCredentialProvider.dll
```

v1 目标是“已登录会话锁定后的 BLE 解锁”。Provider 只有在 `%ProgramData%\ShadowGate\credential_auth.json` 中存在未过期 BLE 授权，且 Credential Manager 中存在 `ShadowGate:WindowsUnlock` 凭据时才会提交解锁凭据；其余情况均 fail closed。

### 构建 Android APK

```bash
cd android-app
./gradlew assembleRelease
```

## 关键配置

| 参数 | 默认值 | 说明 |
|---|---|---|
| `rssi.unlock_threshold_dbm` | -60 | 触发解锁的信号强度 |
| `rssi.lock_threshold_dbm` | -80 | 触发锁定的信号强度 |
| `rssi.hysteresis_samples` | 3 | 防抖连续采样数 |
| `challenge.timeout_ms` | 1500 | 质询超时(防范中继攻击) |
| `scanning.scan_interval_ms` | 1000 | BLE 扫描间隔 |

## 安全设计

1. **Ed25519 非对称签名**: 私钥永不离开设备，仅对质询签名
2. **物理时间限制**: Challenge 超时 1500ms，阻止中继攻击
3. **双向不对称迟滞**: 解锁需强信号，锁定需弱信号 + 持续时间
4. **防重放**: 每次质询使用新鲜随机数 + 序列号

## License

MIT — © 2026 Nous Research
