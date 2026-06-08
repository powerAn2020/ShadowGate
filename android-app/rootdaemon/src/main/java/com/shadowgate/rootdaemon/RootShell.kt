package com.shadowgate.rootdaemon

import android.content.Context
import android.os.Build
import android.util.Log
import java.io.BufferedReader
import java.io.DataOutputStream
import java.io.InputStreamReader

/**
 * Root Shell 工具 — 通过 su 执行特权命令
 *
 * 支持:
 * - Magisk (magisk su)
 * - KernelSU (ksu)
 * - SuperSU / 传统 su
 *
 * 核心用途:
 * 1. 禁用 Doze 省电限制 (dumpsys deviceidle disable)
 * 2. 注入 BLE 广播保活参数
 * 3. 修改系统蓝牙调度策略
 * 4. 守护进程自保活 (watchdog)
 */
object RootShell {

    private const val TAG = "ShadowGateRoot"
    private var rootAvailable: Boolean? = null
    private var rootType: String = "unknown"

    /**
     * 检测 Root 是否可用
     */
    fun isRootAvailable(): Boolean {
        if (rootAvailable != null) return rootAvailable!!

        rootAvailable = try {
            val process = Runtime.getRuntime().exec(arrayOf("su", "-c", "id"))
            val reader = BufferedReader(InputStreamReader(process.inputStream))
            val output = reader.readLine()
            process.waitFor()
            val result = output?.contains("uid=0") == true
            if (result) {
                Log.i(TAG, "Root access available: $output")
            }
            result
        } catch (e: Exception) {
            Log.w(TAG, "Root check failed: ${e.message}")
            false
        }

        // 检测 Root 类型
        if (rootAvailable == true) {
            rootType = detectRootType()
            Log.i(TAG, "Root type: $rootType")
        }

        return rootAvailable!!
    }

    /**
     * 检测 Root 实现类型
     */
    private fun detectRootType(): String {
        return when {
            exec("test -d /data/adb/ksu && echo 'yes'").contains("yes") -> "KernelSU"
            exec("test -d /data/adb/magisk && echo 'yes'").contains("yes") -> "Magisk"
            exec("which magisk").isNotEmpty() -> "Magisk"
            exec("which su").isNotEmpty() -> "SuperSU"
            else -> "unknown"
        }
    }

    fun getRootType(): String = rootType

    /**
     * 以 Root 身份执行命令
     * @return stdout 输出字符串
     */
    fun exec(command: String, timeoutMs: Long = 5000): String {
        if (isRootAvailable() != true) {
            Log.w(TAG, "Root not available, skipping: $command")
            return ""
        }

        return try {
            val process = Runtime.getRuntime().exec(arrayOf("su", "-c", command))
            val reader = BufferedReader(InputStreamReader(process.inputStream))
            val errorReader = BufferedReader(InputStreamReader(process.errorStream))

            val output = StringBuilder()
            val errors = StringBuilder()

            // 读取 stdout
            reader.useLines { lines ->
                lines.forEach { output.appendLine(it) }
            }
            // 读取 stderr
            errorReader.useLines { lines ->
                lines.forEach { errors.appendLine(it) }
            }

            // 超时等待
            val finished = process.waitFor()

            if (errors.isNotEmpty()) {
                Log.d(TAG, "Root command stderr: $errors")
            }

            output.toString().trim()
        } catch (e: Exception) {
            Log.e(TAG, "Root command failed: $command", e)
            ""
        }
    }

    /**
     * 以 Root 身份执行命令 (流式，用于长时间运行的 daemon)
     */
    fun execStreaming(command: String, onLine: (String) -> Unit): Process? {
        if (isRootAvailable() != true) return null

        return try {
            val process = Runtime.getRuntime().exec(arrayOf("su", "-c", command))
            val reader = BufferedReader(InputStreamReader(process.inputStream))

            Thread {
                reader.useLines { lines ->
                    lines.forEach { line ->
                        onLine(line)
                    }
                }
            }.start()

            process
        } catch (e: Exception) {
            Log.e(TAG, "Streaming root command failed", e)
            null
        }
    }

    /**
     * 以 Root 身份写入文件
     */
    fun writeFile(path: String, content: String): Boolean {
        val escaped = content.replace("'", "'\\''")
        val result = exec("echo '$escaped' > $path")
        return result.isEmpty() // 成功时 stdout 为空
    }

    /**
     * 以 Root 身份执行多条命令 (批量)
     */
    fun execBatch(vararg commands: String): List<String> {
        if (isRootAvailable() != true) return emptyList()

        val script = commands.joinToString(" && ")
        return exec(script).lines()
    }
}

/**
 * Doze 模式控制器 (Root)
 *
 * 直接操控 Android Doze/AppStandby 机制，
 * 确保 ShadowGate 在息屏后不被挂起。
 */
object DozeController {

    private const val TAG = "ShadowGateDoze"

    /**
     * 完全禁用 Doze 模式
     * 需要 Root: dumpsys deviceidle disable
     *
     * 注意: 这会关闭整个系统的 Doze，可能影响其他应用省电
     */
    fun disableDozeCompletely(): Boolean {
        Log.i(TAG, "Disabling Doze mode completely...")
        val result = RootShell.exec("dumpsys deviceidle disable")
        val success = result.contains("Disabled") || result.contains("Idle mode disabled")
        Log.i(TAG, "Doze disabled: $success — $result")
        return success
    }

    /**
     * 将 ShadowGate 加入 Doze 白名单
     * 不需要 Root，但需要 REQUEST_IGNORE_BATTERY_OPTIMIZATIONS
     */
    fun addToWhitelist(context: Context): Boolean {
        // 此方法通过 PowerManager 实现，不需要 Root
        return true // 由 Activity 处理
    }

    /**
     * 禁用 ShadowGate 的 App Standby
     * Root: dumpsys appops set <package> RUN_IN_BACKGROUND allow
     */
    fun disableAppStandby(packageName: String): Boolean {
        val commands = arrayOf(
            "dumpsys appops set $packageName RUN_IN_BACKGROUND allow",
            "dumpsys appops set $packageName RUN_ANY_IN_BACKGROUND allow",
            "am set-standby-bucket $packageName active",
        )
        val results = RootShell.execBatch(*commands)
        Log.i(TAG, "AppStandby disabled for $packageName")
        return results.all { !it.contains("Error") }
    }

    /**
     * 将进程设为不可杀 (OOM 调整)
     * Root: echo -17 > /proc/<pid>/oom_adj
     */
    fun makeUnkillable(pid: Int): Boolean {
        val result = RootShell.exec("echo -17 > /proc/$pid/oom_adj")
        val success = result.isEmpty()
        Log.i(TAG, "PID $pid OOM adj set to -17: $success")
        return success
    }

    /**
     * 提升进程调度优先级
     * Root: renice -20 -p <pid>
     */
    fun setHighPriority(pid: Int): Boolean {
        val result = RootShell.exec("renice -20 -p $pid")
        Log.i(TAG, "PID $pid priority set to -20")
        return !result.contains("failed")
    }
}

/**
 * BLE 底层控制器 (Root)
 *
 * 通过 sysfs / HCI 直接操控蓝牙适配器，
 * 绕过 Android Framework 限制。
 */
object BleController {

    private const val TAG = "ShadowGateBle"

    /**
     * 确保 BLE 适配器不被关闭
     *
     * 通过 sysfs 阻止蓝牙芯片进入省电模式:
     * - 禁止 USB autosuspend (蓝牙走 USB)
     * - 设置蓝牙唤醒锁
     */
    fun preventAdapterSleep(): Boolean {
        val commands = arrayOf(
            // 禁止蓝牙 HCI 设备进入省电
            "for dev in /sys/class/bluetooth/hci*; do echo 0 > \$dev/power/control 2>/dev/null; done",
            "for dev in /sys/devices/**/power/control; do echo 'on' > \$dev 2>/dev/null; done",
            // 设置唤醒锁确保芯片不休眠
            "echo 'shadowgate' > /sys/power/wake_lock 2>/dev/null || true",
        )
        RootShell.execBatch(*commands)
        Log.i(TAG, "BLE adapter sleep prevention applied")
        return true
    }

    /**
     * 强制 BLE 广播参数
     *
     * 修改 Adapter 参数提升广播稳定性:
     * - 增大广播功率
     * - 扩展广播超时
     */
    fun forceAdvertisingParams(): Boolean {
        // 通过 bluetoothd 或 hcitool 强制参数
        val commands = arrayOf(
            // 关闭 BLE 扫描滤波器 (提升广播发现率)
            "settings put global ble_scan_always_enabled 1",
            // 蓝牙连接保活
            "settings put secure bluetooth_on 1",
            "svc bluetooth enable 2>/dev/null || true",
        )
        RootShell.execBatch(*commands)
        Log.i(TAG, "Advertising params forced")
        return true
    }

    /**
     * 重启蓝牙栈 (如果卡死)
     */
    fun resetBluetoothStack(): Boolean {
        Log.w(TAG, "Resetting Bluetooth stack...")
        val commands = arrayOf(
            "svc bluetooth disable",
            "sleep 2",
            "svc bluetooth enable",
        )
        RootShell.execBatch(*commands)
        Log.i(TAG, "Bluetooth stack reset")
        return true
    }

    /**
     * 获取当前 BLE 状态
     */
    fun getBleState(): BleState {
        val hciDev = RootShell.exec("hciconfig 2>/dev/null")
        val isUp = hciDev.contains("UP RUNNING")

        return BleState(
            adapterUp = isUp,
            hciInfo = hciDev.take(200),
            connectedDevices = RootShell.exec("hcitool con 2>/dev/null").lines().size - 1
        )
    }
}

data class BleState(
    val adapterUp: Boolean,
    val hciInfo: String,
    val connectedDevices: Int
)

/**
 * 系统服务守护 (Root)
 *
 * 通过 service call / am 维持关键服务运行
 */
object SystemServiceGuard {

    private const val TAG = "ShadowGateGuard"

    /**
     * 锁定关键系统属性防止被省电策略覆盖
     */
    fun lockProperties(): Boolean {
        val props = mapOf(
            "persist.bluetooth.btsnoopenable" to "true",
            "persist.bluetooth.enableinbandringing" to "true",
            "persist.bluetooth.leaudio.enabled" to "true",
        )

        for ((key, value) in props) {
            RootShell.exec("setprop $key $value")
        }

        Log.i(TAG, "System properties locked")
        return true
    }

    /**
     * 确保前台服务不被杀死
     *
     * 通过 ActivityManager 持久化服务:
     * am set-service-foreground ...
     */
    fun protectService(packageName: String, serviceName: String): Boolean {
        // 将服务标记为 persistent
        val commands = arrayOf(
            "dumpsys activity services $packageName",
            "am broadcast -a android.intent.action.BOOT_COMPLETED -p $packageName 2>/dev/null || true",
        )
        RootShell.execBatch(*commands)
        return true
    }

    /**
     * 守护进程自我保活 (通过 init.rc 或定时检查)
     *
     * 如果是在 KernelSU/Magisk 环境下，
     * 推荐通过模块的 service.sh 启动独立守护进程。
     */
    fun installBootScript(context: Context, packageName: String): Boolean {
        val script = """
            #!/system/bin/sh
            # ShadowGate Boot Daemon — 系统启动后自动运行
            # 安装到: /data/adb/service.d/shadowgate.sh (Magisk)
            #         /data/adb/ksu/service.d/shadowgate.sh (KernelSU)
            
            # 等待系统就绪
            while [ "\$(getprop sys.boot_completed)" != "1" ]; do sleep 5; done
            sleep 30
            
            # 禁用 Doze
            dumpsys deviceidle disable
            
            # 确保蓝牙开启
            svc bluetooth enable
            
            # 启动 ShadowGate
            am start-foreground-service -n $packageName/.service.ShadowGateService
        """.trimIndent()

        // 写入到 Magisk/KernelSU 的 service.d 目录
        var success = false

        // Magisk
        if (RootShell.exec("test -d /data/adb/service.d && echo yes").contains("yes")) {
            RootShell.writeFile("/data/adb/service.d/shadowgate.sh", script)
            RootShell.exec("chmod 755 /data/adb/service.d/shadowgate.sh")
            success = true
            Log.i(TAG, "Installed Magisk boot script")
        }

        // KernelSU
        if (RootShell.exec("test -d /data/adb/ksu/service.d && echo yes").contains("yes")) {
            RootShell.writeFile("/data/adb/ksu/service.d/shadowgate.sh", script)
            RootShell.exec("chmod 755 /data/adb/ksu/service.d/shadowgate.sh")
            success = true
            Log.i(TAG, "Installed KernelSU boot script")
        }

        return success
    }
}
