package com.shadowgate.app

import android.app.Application
import android.app.NotificationChannel
import android.app.NotificationManager
import android.os.Build
import com.shadowgate.app.crypto.NativeCrypto
import com.shadowgate.app.service.ShadowGateService

class ShadowGateApp : Application() {

    companion object {
        const val CHANNEL_ID = "shadowgate_foreground"
        const val CHANNEL_NAME = "ShadowGate 近场守护"
        const val NOTIFICATION_ID = 1001
    }

    override fun onCreate() {
        super.onCreate()

        createNotificationChannel()
        NativeCrypto.init()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                CHANNEL_NAME,
                NotificationManager.IMPORTANCE_LOW  // 低优先级，不打扰用户
            ).apply {
                description = "ShadowGate BLE 近场锁屏服务正在运行"
                setShowBadge(false)
            }

            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(channel)
        }
    }
}
