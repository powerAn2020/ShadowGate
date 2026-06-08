//! JNI FFI 接口 — 暴露 Rust Core 功能给 Android (Kotlin/Java)
//!
//! 此模块仅在 `target_os = "android"` 时编译。
//! 通过 `cdylib` 输出编译为 `.so` 文件供 Android 加载。

#[cfg(target_os = "android")]
mod android_impl {
    use jni::objects::{JByteArray, JClass, JString};
    use jni::sys::{jboolean, jbyteArray, jstring};
    use jni::JNIEnv;

    use crate::crypto::KeyPair;
    use crate::protocol;
    use crate::rssi_filter::{
        HysteresisAction, HysteresisDetector, KalmanFilter, MovingAverageFilter,
    };

    // ===== 密钥管理 =====

    /// 生成密钥对，返回公钥字节数组 (32 bytes)
    ///
    /// Kotlin 调用: `NativeLib.generateKeyPair(): ByteArray`
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_generateKeyPair(
        mut env: JNIEnv,
        _class: JClass,
    ) -> jbyteArray {
        let keypair = KeyPair::generate().expect("key generation failed");
        let seed = keypair.secret_seed();
        let result: Vec<u8> = seed.to_vec();

        env.byte_array_from_slice(&result)
            .expect("failed to create byte array")
            .into_raw()
    }

    /// 从种子恢复密钥对并签名
    ///
    /// @param seed 32 字节私钥种子
    /// @param message 待签名消息
    /// @return 64 字节签名
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_sign(
        mut env: JNIEnv,
        _class: JClass,
        seed: JByteArray,
        message: JByteArray,
    ) -> jbyteArray {
        let seed_bytes: Vec<u8> = env.convert_byte_array(&seed).expect("invalid seed array");

        let seed_arr: [u8; 32] = seed_bytes
            .as_slice()
            .try_into()
            .expect("seed must be 32 bytes");

        let message_bytes: Vec<u8> = env
            .convert_byte_array(&message)
            .expect("invalid message array");

        let keypair = KeyPair::from_bytes(&seed_arr).expect("invalid seed");
        let signature = keypair.sign(&message_bytes);

        env.byte_array_from_slice(&signature.bytes)
            .expect("failed to create byte array")
            .into_raw()
    }

    /// 验证签名
    ///
    /// @param public_key 32 字节公钥
    /// @param message 原始消息
    /// @param signature 64 字节签名
    /// @return true 验证通过
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_verify(
        mut env: JNIEnv,
        _class: JClass,
        public_key: JByteArray,
        message: JByteArray,
        signature: JByteArray,
    ) -> jboolean {
        let pk_bytes: Vec<u8> = env.convert_byte_array(&public_key).unwrap_or_default();
        let msg_bytes: Vec<u8> = env.convert_byte_array(&message).unwrap_or_default();
        let sig_bytes: Vec<u8> = env.convert_byte_array(&signature).unwrap_or_default();

        let pk_arr: [u8; 32] = match pk_bytes.as_slice().try_into() {
            Ok(a) => a,
            Err(_) => return 0,
        };
        let sig_arr: [u8; 64] = match sig_bytes.as_slice().try_into() {
            Ok(a) => a,
            Err(_) => return 0,
        };

        let pk = match crate::crypto::PublicKey::from_bytes(pk_arr) {
            Ok(pk) => pk,
            Err(_) => return 0,
        };

        let sig = crate::crypto::SignatureBytes { bytes: sig_arr };

        pk.verify(&msg_bytes, &sig).unwrap_or(false) as jboolean
    }

    // ===== RSSI 滤波 =====

    /// 创建卡尔曼滤波器并返回指针 (opaque handle)
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_createKalmanFilter(
        initial_rssi: f64,
        process_noise: f64,
        measurement_noise: f64,
    ) -> i64 {
        let filter = KalmanFilter::new(initial_rssi, process_noise, measurement_noise);
        Box::into_raw(Box::new(filter)) as i64
    }

    /// 更新卡尔曼滤波器
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_updateKalmanFilter(
        ptr: i64,
        measurement: f64,
    ) -> f64 {
        let filter = unsafe { &mut *(ptr as *mut KalmanFilter) };
        filter.update(measurement)
    }

    /// 销毁卡尔曼滤波器
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_destroyKalmanFilter(
        ptr: i64,
    ) {
        if ptr != 0 {
            unsafe {
                let _ = Box::from_raw(ptr as *mut KalmanFilter);
            }
        }
    }

    // ===== 迟滞检测 =====

    /// 创建迟滞检测器
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_createHysteresisDetector(
        unlock_threshold: f64,
        lock_threshold: f64,
        unlock_samples: i32,
        lock_samples: i32,
    ) -> i64 {
        let detector = HysteresisDetector::new(
            unlock_threshold,
            lock_threshold,
            unlock_samples as usize,
            lock_samples as usize,
        );
        Box::into_raw(Box::new(detector)) as i64
    }

    /// 更新迟滞检测器，返回动作: 0=None, 1=Unlock, 2=Lock
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_updateHysteresis(
        ptr: i64,
        rssi: f64,
    ) -> i32 {
        let detector = unsafe { &mut *(ptr as *mut HysteresisDetector) };
        match detector.update(rssi) {
            HysteresisAction::None => 0,
            HysteresisAction::Unlock => 1,
            HysteresisAction::Lock => 2,
        }
    }

    /// 销毁迟滞检测器
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_destroyHysteresisDetector(
        ptr: i64,
    ) {
        if ptr != 0 {
            unsafe {
                let _ = Box::from_raw(ptr as *mut HysteresisDetector);
            }
        }
    }

    // ===== 协议序列化 =====

    /// 序列化质询响应
    #[no_mangle]
    pub extern "system" fn Java_com_shadowgate_app_crypto_NativeCrypto_createChallengeResponse(
        mut env: JNIEnv,
        _class: JClass,
        signature: JByteArray,
        sequence: i32,
        device_rssi: i8,
    ) -> jbyteArray {
        let sig_bytes: Vec<u8> = env.convert_byte_array(&signature).unwrap_or_default();
        let sig_arr: [u8; 64] = match sig_bytes.as_slice().try_into() {
            Ok(a) => a,
            Err(_) => {
                return env.byte_array_from_slice(&[]).expect("").into_raw();
            }
        };

        let sig = crate::crypto::SignatureBytes { bytes: sig_arr };
        let data = protocol::create_challenge_response(&sig, sequence as u32, device_rssi)
            .unwrap_or_default();

        env.byte_array_from_slice(&data)
            .expect("failed to create byte array")
            .into_raw()
    }
}

// Non-Android stub
#[cfg(not(target_os = "android"))]
pub mod stub {
    // Placeholder for non-Android builds
}
