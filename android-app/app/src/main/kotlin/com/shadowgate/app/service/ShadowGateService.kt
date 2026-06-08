package com.shadowgate.app.service

import android.app.PendingIntent
import android.app.Service
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.BluetoothLeAdvertiser
import android.content.Context
import android.content.Intent
import android.os.IBinder
import android.os.ParcelUuid
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import com.shadowgate.app.ShadowGateApp
import com.shadowgate.app.crypto.KeyManager
import com.shadowgate.app.crypto.NativeCrypto
import com.shadowgate.app.rootdaemon.RootShell
import com.shadowgate.app.ui.MainActivity
import kotlinx.coroutines.*
import java.util.UUID

/**
 * ShadowGate BLE 前台服务
 *
 * 职责:
 * 1. 作为 BLE 外设 (Peripheral) 持续广播
 * 2. 运行 GATT Server 等待 PC 连接
 * 3. 接收 Challenge 并签名响应
 * 4. 保活 — 前台服务 + WakeLock
 */
class ShadowGateService : Service() {

    companion object {
        private const val TAG = "ShadowGate"

        // BLE Service & Characteristic UUIDs
        val SERVICE_UUID = UUID.fromString("0000shadow-0000-1000-8000-00805f9b34fb")
        val CHAR_CHALLENGE_UUID = UUID.fromString("0000chall-0000-1000-8000-00805f9b34fb")
        val CHAR_RESPONSE_UUID = UUID.fromString("0000resp-0000-1000-8000-00805f9b34fb")
        val CHAR_DEVICE_ID_UUID = UUID.fromString("0000devid-0000-1000-8000-00805f9b34fb")

        // Actions
        const val ACTION_START = "com.shadowgate.action.START"
        const val ACTION_STOP = "com.shadowgate.action.STOP"

        // Config keys
        const val PREF_ADVERTISE_INTERVAL = "advertise_interval_ms"
        const val PREF_TX_POWER = "tx_power"
    }

    private lateinit var bluetoothAdapter: BluetoothAdapter
    private lateinit var advertiser: BluetoothLeAdvertiser
    private lateinit var gattServer: BluetoothGattServer
    private lateinit var keyManager: KeyManager
    private lateinit var wakeLock: PowerManager.WakeLock

    private val serviceScope = CoroutineScope(Dispatchers.Default + SupervisorJob())
    private var isAdvertising = false

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "ShadowGateService onCreate")

        val btManager = getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager
        bluetoothAdapter = btManager.adapter
        advertiser = bluetoothAdapter.bluetoothLeAdvertiser
        keyManager = KeyManager(this)

        // 获取 WakeLock 防止 CPU 休眠
        val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = powerManager.newWakeLock(
            PowerManager.PARTIAL_WAKE_LOCK,
            "ShadowGate:BleService"
        )
        wakeLock.acquire(10 * 60 * 1000L) // 10 min timeout, 会被 renew
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> startForegroundService()
            ACTION_STOP -> stopForegroundService()
            else -> startForegroundService()
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun startForegroundService() {
        // 前台通知
        val pendingIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val notification = NotificationCompat.Builder(this, ShadowGateApp.CHANNEL_ID)
            .setContentTitle("ShadowGate 近场守护")
            .setContentText("BLE 凭证广播中...")
            .setSmallIcon(android.R.drawable.ic_lock_idle_lock)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

        startForeground(ShadowGateApp.NOTIFICATION_ID, notification)

        // 生成密钥对 (如果还没有)
        if (keyManager.getSeed() == null) {
            Log.i(TAG, "Generating new key pair...")
            keyManager.generateAndStore()
        }

        // 启动 GATT Server
        startGattServer()

        // 启动 BLE 广播
        startAdvertising()

        // ★ Root 特权增强: 如果有 Root 权限，应用底层保活措施
        tryApplyRootEnhancements()

        // 定期 renew WakeLock
        serviceScope.launch {
            while (isActive) {
                delay(5 * 60 * 1000L) // 每 5 分钟
                if (wakeLock.isHeld) {
                    wakeLock.acquire(10 * 60 * 1000L)
                }
            }
        }
    }

    /**
     * 如果有 Root 权限，应用底层保活增强
     *
     * Root 模式下直接操控系统参数，绕过 Android 省电限制：
     * - 禁用自己的 App Standby
     * - 设置进程 OOM adj 为 -17 (不可被杀)
     * - 锁定蓝牙适配器不休眠
     * - 同时启动 RootDaemonService 作为独立守护
     */
    private fun tryApplyRootEnhancements() {
        if (!RootShell.isRootAvailable()) {
            Log.i(TAG, "Root not available — using standard foreground service only")
            return
        }

        Log.i(TAG, "Root available (${RootShell.getRootType()}) — applying enhancements...")

        serviceScope.launch {
            try {
                // 系统级保活
                val pkg = packageName

                // 1. 禁用 App Standby
                RootShell.exec("dumpsys appops set $pkg RUN_IN_BACKGROUND allow")
                RootShell.exec("dumpsys appops set $pkg RUN_ANY_IN_BACKGROUND allow")
                RootShell.exec("am set-standby-bucket $pkg active")

                // 2. 锁定蓝牙适配器
                RootShell.exec("settings put global ble_scan_always_enabled 1")
                RootShell.exec("svc bluetooth enable 2>/dev/null || true")

                // 3. 禁止蓝牙芯片省电
                RootShell.exec("for dev in /sys/class/bluetooth/hci*/power/control; do echo 'on' > \$dev 2>/dev/null; done")

                // 4. 设置当前进程不可杀
                val pid = android.os.Process.myPid()
                RootShell.exec("echo -17 > /proc/$pid/oom_adj 2>/dev/null || true")
                RootShell.exec("renice -20 -p $pid 2>/dev/null || true")

                // 5. 启动独立 RootDaemonService 进程作 watchdog
                startRootDaemonIfNeeded()

                // 6. 安装开机启动脚本 (仅首次)
                installBootScriptIfNeeded()

                Log.i(TAG, "Root enhancements applied successfully")
            } catch (e: Exception) {
                Log.e(TAG, "Root enhancement failed", e)
            }
        }
    }

    /**
     * 启动 RootDaemonService (在独立进程)
     */
    private fun startRootDaemonIfNeeded() {
        try {
            val intent = Intent().apply {
                setClassName(packageName, "com.shadowgate.app.rootdaemon.RootDaemonService")
            }
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
                startForegroundService(intent)
            } else {
                startService(intent)
            }
            Log.i(TAG, "RootDaemonService started in separate process")
        } catch (e: Exception) {
            Log.w(TAG, "Could not start RootDaemonService: ${e.message}")
        }
    }

    /**
     * 安装 Magisk/KernelSU 开机启动脚本
     */
    private fun installBootScriptIfNeeded() {
        val script = """
#!/system/bin/sh
# ShadowGate Auto-Start (installed by ShadowGateService)
while [ "\$(getprop sys.boot_completed)" != "1" ]; do sleep 3; done
sleep 15
dumpsys deviceidle disable
svc bluetooth enable
sleep 5
am start-foreground-service -n $packageName/.service.ShadowGateService
        """.trimIndent()

        // Magisk service.d
        if (RootShell.exec("test -d /data/adb/service.d && echo yes").contains("yes")) {
            RootShell.writeFile("/data/adb/service.d/shadowgate.sh", script)
            RootShell.exec("chmod 755 /data/adb/service.d/shadowgate.sh")
            Log.i(TAG, "Boot script installed (Magisk)")
        }

        // KernelSU service.d
        if (RootShell.exec("test -d /data/adb/ksu/service.d && echo yes").contains("yes")) {
            RootShell.writeFile("/data/adb/ksu/service.d/shadowgate.sh", script)
            RootShell.exec("chmod 755 /data/adb/ksu/service.d/shadowgate.sh")
            Log.i(TAG, "Boot script installed (KernelSU)")
        }
    }

    private fun stopForegroundService() {
        stopAdvertising()
        stopGattServer()
        wakeLock.let { if (it.isHeld) it.release() }
        serviceScope.cancel()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    // ===== GATT Server =====

    private fun startGattServer() {
        gattServer = bluetoothManager.openGattServer(this, object : BluetoothGattServerCallback() {

            override fun onConnectionStateChange(device: android.bluetooth.BluetoothDevice, status: Int, newState: Int) {
                Log.i(TAG, "GATT connection: device=${device.address}, state=$newState")
            }

            override fun onCharacteristicWriteRequest(
                device: android.bluetooth.BluetoothDevice,
                requestId: Int,
                characteristic: BluetoothGattCharacteristic,
                preparedWrite: Boolean,
                responseNeeded: Boolean,
                offset: Int,
                value: ByteArray
            ) {
                when (characteristic.uuid) {
                    CHAR_CHALLENGE_UUID -> {
                        handleChallenge(device, requestId, value)
                    }
                    else -> {
                        gattServer.sendResponse(device, requestId, BluetoothGattCharacteristic.GATT_FAILURE, 0, null)
                    }
                }
            }
        })

        val service = BluetoothGattService(SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY).apply {
            // Challenge Characteristic (Write)
            addCharacteristic(BluetoothGattCharacteristic(
                CHAR_CHALLENGE_UUID,
                BluetoothGattCharacteristic.PROPERTY_WRITE,
                BluetoothGattCharacteristic.PERMISSION_WRITE
            ))

            // Response Characteristic (Read + Notify)
            addCharacteristic(BluetoothGattCharacteristic(
                CHAR_RESPONSE_UUID,
                BluetoothGattCharacteristic.PROPERTY_READ or BluetoothGattCharacteristic.PROPERTY_NOTIFY,
                BluetoothGattCharacteristic.PERMISSION_READ
            ))

            // Device ID Characteristic (Read)
            val deviceHash = keyManager.getOrCreateDeviceHash()
            addCharacteristic(BluetoothGattCharacteristic(
                CHAR_DEVICE_ID_UUID,
                BluetoothGattCharacteristic.PROPERTY_READ,
                BluetoothGattCharacteristic.PERMISSION_READ
            ).apply {
                setValue(deviceHash)
            })
        }

        gattServer.addService(service)
        Log.i(TAG, "GATT Server started")
    }

    private val bluetoothManager: BluetoothManager
        get() = getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager

    /**
     * 处理 PC 发来的 Challenge
     */
    private fun handleChallenge(
        device: android.bluetooth.BluetoothDevice,
        requestId: Int,
        challengeData: ByteArray
    ) {
        Log.i(TAG, "Challenge received: ${challengeData.size} bytes from ${device.address}")

        serviceScope.launch {
            try {
                val seed = keyManager.getSeed()
                if (seed == null) {
                    Log.e(TAG, "No key seed available")
                    gattServer.sendResponse(device, requestId, BluetoothGattCharacteristic.GATT_FAILURE, 0, null)
                    return@launch
                }

                // 用私钥签名 Challenge
                val signature = NativeCrypto.sign(seed, challengeData)

                // 构造响应
                val sequence = if (challengeData.size >= 4) {
                    // 从 challenge 中提取序列号
                    challengeData.takeLast(4).fold(0) { acc, b -> (acc shl 8) or (b.toInt() and 0xFF) }
                } else 0

                val response = NativeCrypto.createChallengeResponse(signature, sequence, 0)

                // 发送响应 (通过 Notification)
                val responseChar = gattServer.getService(SERVICE_UUID)
                    ?.getCharacteristic(CHAR_RESPONSE_UUID)
                responseChar?.setValue(response)

                gattServer.notifyCharacteristicChanged(device, responseChar, false)

                // 也发送 GATT Write Response 确认收到
                gattServer.sendResponse(device, requestId, BluetoothGattCharacteristic.GATT_SUCCESS, 0, null)

                Log.i(TAG, "Challenge response sent")
            } catch (e: Exception) {
                Log.e(TAG, "Challenge processing failed", e)
                gattServer.sendResponse(device, requestId, BluetoothGattCharacteristic.GATT_FAILURE, 0, null)
            }
        }
    }

    private fun stopGattServer() {
        try {
            gattServer.clearServices()
            gattServer.close()
        } catch (e: Exception) {
            Log.e(TAG, "Error closing GATT server", e)
        }
    }

    // ===== BLE Advertising =====

    private fun startAdvertising() {
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
            .setConnectable(true)
            .setTimeout(0) // 持续广播
            .build()

        val deviceHash = keyManager.getOrCreateDeviceHash()

        // Advertise Data: 嵌入 DeviceInfo (服务数据和制造商数据)
        val advertiseData = AdvertiseData.Builder()
            .setIncludeDeviceName(false)
            .addServiceUuid(ParcelUuid(SERVICE_UUID))
            .addServiceData(ParcelUuid(SERVICE_UUID), deviceHash)
            .build()

        val scanResponse = AdvertiseData.Builder()
            .setIncludeDeviceName(true)
            .build()

        advertiser.startAdvertising(settings, advertiseData, scanResponse, object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings) {
                isAdvertising = true
                Log.i(TAG, "BLE Advertising started (mode=${settingsInEffect.mode}, tx=${settingsInEffect.txPowerLevel})")
            }

            override fun onStartFailure(errorCode: Int) {
                isAdvertising = false
                Log.e(TAG, "BLE Advertising failed: error=$errorCode")
            }
        })
    }

    private fun stopAdvertising() {
        advertiser.stopAdvertising(AdvertiseCallback {})
        isAdvertising = false
        Log.i(TAG, "BLE Advertising stopped")
    }

    override fun onDestroy() {
        stopAdvertising()
        stopGattServer()
        wakeLock.let { if (it.isHeld) it.release() }
        serviceScope.cancel()
        super.onDestroy()
    }
}
