package com.shadowgate.rootdaemon

import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import kotlinx.coroutines.*

/**
 * Root 特权守护进程
 *
 * 作为独立的 Android Service 运行，通过 su 获取 root 权限。
 *
 * 核心职责:
 * 1. 禁用 Doze / AppStandby (系统级)
 * 2. 锁定蓝牙适配器不休眠 (sysfs)
 * 3. 守护 BLE 广播服务不被系统杀死 (OOM adj -17)
 * 4. 定期自检 + 自动恢复 (watchdog)
 * 5. 安装开机启动脚本 (Magisk/KernelSU service.d)
 *
 * 与 Xposed 模块的区别:
 * - Xposed: Hook 系统框架，透明拦截，不需要 root daemon
 * - Root: 直接修改系统参数，更直接但也更"粗暴"
 * - Root 方案不依赖 Xposed/LSPosed 环境
 */
class RootDaemonService : Service() {

    companion object {
        private const val TAG = "ShadowGateRootD"
        private const val WATCHDOG_INTERVAL_MS = 30_000L  // 30 秒检查一次
        private const val SHADOWGATE_PACKAGE = "com.shadowgate.app"
        private const val SHADOWGATE_SERVICE = "com.shadowgate.app.service.ShadowGateService"
    }

    private lateinit var watchdogHandler: Handler
    private lateinit var watchdogThread: HandlerThread
    private lateinit var powerManager: PowerManager

    private var isInitialized = false
    private var consecutiveFailures = 0
    private val maxFailures = 3

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "RootDaemonService onCreate")

        powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager

        // 初始化 Root 环境
        initializeRootEnvironment()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.i(TAG, "RootDaemonService starting...")

        if (!isInitialized) {
            initializeRootEnvironment()
        }

        startWatchdog()

        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    /**
     * 初始化 Root 环境 — 一次性系统参数注入
     */
    private fun initializeRootEnvironment() {
        if (isInitialized) return

        if (!RootShell.isRootAvailable()) {
            Log.e(TAG, "No root access! Root daemon cannot function.")
            return
        }

        Log.i(TAG, "Initializing root environment (${RootShell.getRootType()})...")

        // 1. 禁用 Doze (全局)
        DozeController.disableDozeCompletely()

        // 2. 禁用 ShadowGate 的 App Standby
        DozeController.disableAppStandby(SHADOWGATE_PACKAGE)

        // 3. 锁定系统属性
        SystemServiceGuard.lockProperties()

        // 4. 禁止蓝牙适配器休眠
        BleController.preventAdapterSleep()

        // 5. 强制 BLE 广播参数
        BleController.forceAdvertisingParams()

        // 6. 安装开机启动脚本
        SystemServiceGuard.installBootScript(this, SHADOWGATE_PACKAGE)

        // 7. 标记当前进程为不可杀
        DozeController.makeUnkillable(android.os.Process.myPid())
        DozeController.setHighPriority(android.os.Process.myPid())

        isInitialized = true
        Log.i(TAG, "Root environment initialized successfully")
    }

    /**
     * 启动看门狗 — 定期检查系统状态并自动修复
     */
    private fun startWatchdog() {
        watchdogThread = HandlerThread("ShadowGateWatchdog").apply { start() }
        watchdogHandler = Handler(watchdogThread.looper)

        val watchdogRunnable = object : Runnable {
            override fun run() {
                try {
                    checkAndRepair()
                } catch (e: Exception) {
                    Log.e(TAG, "Watchdog check failed", e)
                    consecutiveFailures++
                }

                if (consecutiveFailures < maxFailures) {
                    watchdogHandler.postDelayed(this, WATCHDOG_INTERVAL_MS)
                } else {
                    Log.e(TAG, "Too many failures — stopping watchdog")
                }
            }
        }

        watchdogHandler.post(watchdogRunnable)
        Log.i(TAG, "Watchdog started (interval=${WATCHDOG_INTERVAL_MS}ms)")
    }

    /**
     * 检查系统状态并自动修复
     */
    private fun checkAndRepair() {
        Log.d(TAG, "Watchdog checking...")

        // 1. 检查 Doze 状态
        checkDozeState()

        // 2. 检查蓝牙适配器
        val bleState = BleController.getBleState()
        if (!bleState.adapterUp) {
            Log.w(TAG, "Bluetooth adapter DOWN — attempting reset...")
            BleController.resetBluetoothStack()
            BleController.preventAdapterSleep()
        }

        // 3. 检查 ShadowGate 服务是否在运行
        val serviceRunning = checkServiceRunning()
        if (!serviceRunning) {
            Log.w(TAG, "ShadowGateService not running — restarting...")
            restartShadowGateService()
        }

        // 4. 重新确保 OOM 不被杀死
        // 检查自己进程的 OOM adj
        val oomAdj = getOomAdj()
        if (oomAdj > -10) {
            Log.w(TAG, "OOM adj drifted to $oomAdj — fixing...")
            DozeController.makeUnkillable(android.os.Process.myPid())
        }

        consecutiveFailures = 0
        Log.d(TAG, "Watchdog check passed (BLE=${bleState.adapterUp}, Service=$serviceRunning, OOM=$oomAdj)")
    }

    /**
     * 检查 Doze 是否活跃
     */
    private fun checkDozeState() {
        val result = RootShell.exec("dumpsys deviceidle | grep mState")
        if (result.contains("ACTIVE") || result.contains("IDLE")) {
            Log.w(TAG, "Doze active — re-disabling... ($result)")
            DozeController.disableDozeCompletely()
        }
    }

    /**
     * 检查 ShadowGateService 是否在运行
     */
    private fun checkServiceRunning(): Boolean {
        val result = RootShell.exec("dumpsys activity services $SHADOWGATE_PACKAGE")
        return result.contains("ShadowGateService") &&
               (result.contains("isForeground=true") || result.contains("startRequested=true"))
    }

    /**
     * 重启 ShadowGateService
     */
    private fun restartShadowGateService() {
        val intent = Intent().apply {
            setClassName(SHADOWGATE_PACKAGE, SHADOWGATE_SERVICE)
            action = "com.shadowgate.action.START"
        }

        try {
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
                startForegroundService(intent)
            } else {
                startService(intent)
            }
            Log.i(TAG, "ShadowGateService restart issued")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to restart ShadowGateService", e)

            // Root fallback: am 命令直接启动
            RootShell.exec("am start-foreground-service -n $SHADOWGATE_PACKAGE/$SHADOWGATE_SERVICE")
        }
    }

    /**
     * 获取当前进程的 OOM adj 值
     */
    private fun getOomAdj(): Int {
        val result = RootShell.exec("cat /proc/${android.os.Process.myPid()}/oom_adj")
        return result.trim().toIntOrNull() ?: 0
    }

    /**
     * 获取看门狗状态报告
     */
    fun getStatusReport(): RootDaemonStatus {
        val bleState = BleController.getBleState()
        return RootDaemonStatus(
            rootAvailable = RootShell.isRootAvailable(),
            rootType = RootShell.getRootType(),
            bleUp = bleState.adapterUp,
            serviceRunning = checkServiceRunning(),
            oomAdj = getOomAdj(),
            consecutiveFailures = consecutiveFailures,
            initialized = isInitialized,
            connectedBleDevices = bleState.connectedDevices,
        )
    }

    override fun onDestroy() {
        Log.i(TAG, "RootDaemonService onDestroy")
        watchdogThread.quitSafely()
        super.onDestroy()
    }

    data class RootDaemonStatus(
        val rootAvailable: Boolean,
        val rootType: String,
        val bleUp: Boolean,
        val serviceRunning: Boolean,
        val oomAdj: Int,
        val consecutiveFailures: Int,
        val initialized: Boolean,
        val connectedBleDevices: Int,
    )
}
