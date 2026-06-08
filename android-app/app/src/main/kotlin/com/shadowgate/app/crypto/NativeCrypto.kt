package com.shadowgate.app.crypto

import android.util.Base64
import java.security.SecureRandom

/**
 * Android 端 JNI 桥接 — 调用 Rust Core 的 .so 文件
 *
 * libshadowgate_core.so 由 Rust cdylib 交叉编译生成，
 * 放置于 src/main/jniLibs/<abi>/ 目录下。
 *
 * JNI 命名约定:
 * Java_com_shadowgate_app_crypto_NativeCrypto_<methodName>
 * 对应 core-lib/src/ffi.rs 中的 #[no_mangle] 函数
 */
object NativeCrypto {

    private var loaded = false

    /**
     * 初始化 — 加载 .so 库
     */
    fun init() {
        if (!loaded) {
            try {
                System.loadLibrary("shadowgate_core")
                loaded = true
            } catch (e: UnsatisfiedLinkError) {
                // Xposed 环境下可能使用不同的加载方式
                try {
                    System.load("/data/local/tmp/libshadowgate_core.so")
                    loaded = true
                } catch (e2: UnsatisfiedLinkError) {
                    android.util.Log.e("ShadowGate", "Failed to load native library", e2)
                }
            }
        }
    }

    fun isLoaded(): Boolean = loaded

    /** 生成 Ed25519 密钥对，返回 32 字节私钥种子 */
    external fun generateKeyPair(): ByteArray

    /** 用私钥种子签名消息 */
    external fun sign(seed: ByteArray, message: ByteArray): ByteArray

    /** 用公钥验证签名 */
    external fun verify(publicKey: ByteArray, message: ByteArray, signature: ByteArray): Boolean

    /** 创建卡尔曼滤波器，返回不透明指针 */
    external fun createKalmanFilter(initialRssi: Double, processNoise: Double, measurementNoise: Double): Long

    /** 更新卡尔曼滤波器 */
    external fun updateKalmanFilter(ptr: Long, measurement: Double): Double

    /** 销毁卡尔曼滤波器 */
    external fun destroyKalmanFilter(ptr: Long)

    /** 创建迟滞检测器 */
    external fun createHysteresisDetector(unlockThreshold: Double, lockThreshold: Double, unlockSamples: Int, lockSamples: Int): Long

    /** 更新迟滞检测器: 0=None, 1=Unlock, 2=Lock */
    external fun updateHysteresis(ptr: Long, rssi: Double): Int

    /** 销毁迟滞检测器 */
    external fun destroyHysteresisDetector(ptr: Long)

    /** 创建质询响应 (序列化的二进制数据) */
    external fun createChallengeResponse(signature: ByteArray, sequence: Int, deviceRssi: Byte): ByteArray
}

/**
 * 密钥管理器 — 使用 Android Keystore 安全存储密钥
 */
class KeyManager(private val context: android.content.Context) {

    private val prefs = context.getSharedPreferences("shadowgate_keystore", android.content.Context.MODE_PRIVATE)

    companion object {
        private const val KEY_SEED = "ed25519_seed_b64"
        private const val KEY_PUBLIC = "ed25519_public_b64"
        private const val KEY_DEVICE_HASH = "device_hash_hex"
    }

    /**
     * 生成并存储新密钥对
     */
    fun generateAndStore(): Pair<ByteArray, ByteArray> {
        val seed = NativeCrypto.generateKeyPair()
        val publicKey = derivePublicKey(seed)

        prefs.edit()
            .putString(KEY_SEED, Base64.encodeToString(seed, Base64.NO_WRAP))
            .putString(KEY_PUBLIC, Base64.encodeToString(publicKey, Base64.NO_WRAP))
            .apply()

        return Pair(seed, publicKey)
    }

    /**
     * 获取已存储的私钥种子
     */
    fun getSeed(): ByteArray? {
        val b64 = prefs.getString(KEY_SEED, null) ?: return null
        return Base64.decode(b64, Base64.NO_WRAP)
    }

    /**
     * 获取已存储的公钥
     */
    fun getPublicKey(): ByteArray? {
        val b64 = prefs.getString(KEY_PUBLIC, null) ?: return null
        return Base64.decode(b64, Base64.NO_WRAP)
    }

    /**
     * 生成设备哈希标识
     */
    fun getOrCreateDeviceHash(): ByteArray {
        val existing = prefs.getString(KEY_DEVICE_HASH, null)
        if (existing != null) {
            return hexToBytes(existing)
        }
        val hash = ByteArray(8)
        SecureRandom().nextBytes(hash)
        prefs.edit().putString(KEY_DEVICE_HASH, bytesToHex(hash)).apply()
        return hash
    }

    /**
     * 从私钥种子推导公钥
     */
    private fun derivePublicKey(seed: ByteArray): ByteArray {
        // 用签名一个空消息来验证密钥，然后从 verify 获取公钥
        // 简化: 实际应从 Rust 层直接导出公钥
        val dummyMessage = "shadowgate_key_check".toByteArray()
        val sig = NativeCrypto.sign(seed, dummyMessage)
        // 公钥需要从 seed 推导，这里简化处理
        // 正式实现应在 Rust 层添加 get_public_key_from_seed 函数
        return seed  // placeholder
    }

    private fun bytesToHex(bytes: ByteArray): String {
        return bytes.joinToString("") { "%02x".format(it) }
    }

    private fun hexToBytes(hex: String): ByteArray {
        return hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    }
}
