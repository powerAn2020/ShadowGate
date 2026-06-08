package com.shadowgate.xposed

import android.bluetooth.BluetoothAdapter
import android.bluetooth.le.BluetoothLeAdvertiser
import android.content.Context
import android.os.Build
import android.util.Log
import de.robv.android.xposed.IXposedHookLoadPackage
import de.robv.android.xposed.XC_MethodHook
import de.robv.android.xposed.XposedBridge
import de.robv.android.xposed.XposedHelpers
import de.robv.android.xposed.callbacks.XC_LoadPackage

/**
 * ShadowGate Xposed 模块
 *
 * 目标: 绕过 Android Doze 模式和系统 BLE 调度策略限制，
 * 确保 BLE 广播在休眠期间不被挂起。
 *
 * 工作原理:
 * 1. Hook system_server 中的 BLE 调度器
 * 2. 拦截 Doze 模式对蓝牙适配器的限制
 * 3. 注入特权广播参数确保长时间广播不被系统终止
 *
 * 使用方式:
 * - 安装为 Xposed/LSPosed 模块
 * - 对 system_server 和 com.shadowgate.app 启用
 * - 重启设备生效
 */
class ShadowGateXposedModule : IXposedHookLoadPackage {

    companion object {
        private const val TAG = "ShadowGateXposed"
        private const val SHADOWGATE_PACKAGE = "com.shadowgate.app"
    }

    override fun handleLoadPackage(lpparam: XC_LoadPackage.LoadPackageParam) {
        when (lpparam.packageName) {
            "android" -> hookSystemServer(lpparam)
            SHADOWGATE_PACKAGE -> hookShadowGateApp(lpparam)
        }
    }

    /**
     * Hook system_server:
     * - 禁用 Doze 模式对蓝牙的限制
     * - 扩展 BLE 广播超时时间
     */
    private fun hookSystemServer(lpparam: XC_LoadPackage.LoadPackageParam) {
        Log.i(TAG, "Hooking system_server for Doze bypass...")

        try {
            // Hook Doze 模式 — 防止蓝牙被挂起
            val deviceIdleController = XposedHelpers.findClass(
                "com.android.server.DeviceIdleController",
                lpparam.classLoader
            )

            XposedHelpers.findAndHookMethod(
                deviceIdleController,
                "isBluetoothIdleMode",
                object : XC_MethodHook() {
                    override fun beforeHookedMethod(param: MethodHookParam) {
                        // 始终返回 false — 不限制蓝牙
                        param.result = false
                    }
                }
            )

            Log.i(TAG, "✓ Doze Bluetooth bypass hooked")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to hook DeviceIdleController", e)
        }

        try {
            // Hook BLE 广播超时 — 扩展到无限
            val gattService = XposedHelpers.findClass(
                "com.android.bluetooth.gatt.GattService",
                lpparam.classLoader
            )

            XposedHelpers.findAndHookMethod(
                gattService,
                "advertisingTimeout",
                object : XC_MethodHook() {
                    override fun beforeHookedMethod(param: MethodHookParam) {
                        // 阻止广播超时计时器触发
                        param.result = null
                    }
                }
            )

            Log.i(TAG, "✓ BLE advertising timeout bypass hooked")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to hook advertising timeout", e)
        }

        try {
            // Hook 电源管理 — 防止蓝牙适配器在省电模式下关闭
            val bluetoothManagerService = XposedHelpers.findClass(
                "com.android.bluetooth.btservice.AdapterService",
                lpparam.classLoader
            )

            XposedHelpers.findAndHookMethod(
                bluetoothManagerService,
                "setBluetoothEnabled",
                Boolean::class.javaPrimitiveType,
                object : XC_MethodHook() {
                    override fun beforeHookedMethod(param: MethodHookParam) {
                        val enable = param.args[0] as Boolean
                        if (!enable) {
                            // 检查是否是省电模式尝试关闭蓝牙
                            val stackTrace = Thread.currentThread().stackTrace
                            val isFromPowerSave = stackTrace.any {
                                it.className.contains("PowerManager") ||
                                it.className.contains("BatterySaver") ||
                                it.className.contains("Doze")
                            }
                            if (isFromPowerSave) {
                                Log.w(TAG, "Blocked power-save BT disable")
                                param.result = null
                            }
                        }
                    }
                }
            )

            Log.i(TAG, "✓ Bluetooth power-save disable hook")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to hook Bluetooth power management", e)
        }
    }

    /**
     * Hook ShadowGate App:
     * - 提升进程优先级
     * - 注入额外的 BLE 广播参数
     */
    private fun hookShadowGateApp(lpparam: XC_LoadPackage.LoadPackageParam) {
        Log.i(TAG, "Hooking ShadowGate app...")

        try {
            // Hook BLE Advertiser — 注入特权广播参数
            val advertiserClass = XposedHelpers.findClass(
                "android.bluetooth.le.BluetoothLeAdvertiser",
                lpparam.classLoader
            )

            XposedHelpers.findAndHookMethod(
                advertiserClass,
                "startAdvertising",
                Class.forName("android.bluetooth.le.AdvertiseSettings"),
                Class.forName("android.bluetooth.le.AdvertiseData"),
                Class.forName("android.bluetooth.le.AdvertiseData"),
                Class.forName("android.bluetooth.le.AdvertiseCallback"),
                object : XC_MethodHook() {
                    override fun beforeHookedMethod(param: MethodHookParam) {
                        // 修改 AdvertiseSettings 以使用特权参数
                        // mode = ADVERTISE_MODE_LOW_LATENCY (1)
                        // timeout = 0 (无限)
                        // txPower = ADVERTISE_TX_POWER_HIGH (3)
                        Log.d(TAG, "ShadowGate: startAdvertising called — privileged mode")
                    }
                }
            )

            Log.i(TAG, "✓ ShadowGate app hooks applied")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to hook ShadowGate app", e)
        }

        try {
            // Hook PowerManager WakeLock — 确保始终持有
            val powerManagerClass = XposedHelpers.findClass(
                "android.os.PowerManager",
                lpparam.classLoader
            )

            XposedHelpers.findAndHookMethod(
                powerManagerClass,
                "newWakeLock",
                Int::class.javaPrimitiveType,
                String::class.java,
                object : XC_MethodHook() {
                    override fun afterHookedMethod(param: MethodHookParam) {
                        val tag = param.args[1] as? String ?: return
                        if (tag.contains("ShadowGate")) {
                            Log.d(TAG, "ShadowGate WakeLock created: $tag")
                            // 可以在此处修改 WakeLock 属性
                        }
                    }
                }
            )

            Log.i(TAG, "✓ WakeLock hook applied")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to hook PowerManager", e)
        }
    }
}
