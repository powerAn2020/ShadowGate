/**
 * ShadowGate Core Library — C FFI Header
 *
 * 此头文件定义了 Rust Core 库导出的 C 兼容函数，
 * 供 Android JNI 和可能的其他 FFI 消费者使用。
 *
 * 编译: cargo build --release --target aarch64-linux-android
 * 输出: target/aarch64-linux-android/release/libshadowgate_core.so
 */

#ifndef SHADOWGATE_CORE_H
#define SHADOWGATE_CORE_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// ===== 密钥管理 =====

/** 生成 Ed25519 密钥对 */
int shadowgate_generate_keypair(uint8_t *public_key_out,  // [out] 32 bytes
                                uint8_t *secret_seed_out); // [out] 32 bytes

/** 从种子恢复密钥对 */
int shadowgate_keypair_from_seed(const uint8_t *seed,      // [in] 32 bytes
                                 uint8_t *public_key_out);  // [out] 32 bytes

// ===== 签名与验证 =====

/** 签名消息 */
int shadowgate_sign(const uint8_t *secret_seed,   // [in] 32 bytes
                    const uint8_t *message,       // [in]
                    uint32_t message_len,
                    uint8_t *signature_out);      // [out] 64 bytes

/** 验证签名 */
bool shadowgate_verify(const uint8_t *public_key,      // [in] 32 bytes
                       const uint8_t *message,         // [in]
                       uint32_t message_len,
                       const uint8_t *signature);       // [in] 64 bytes

// ===== RSSI 滤波 =====

/** 卡尔曼滤波器创建 */
void* shadowgate_kalman_create(double initial_rssi,
                               double process_noise,
                               double measurement_noise);

/** 卡尔曼滤波器更新 */
double shadowgate_kalman_update(void *filter, double measurement);

/** 卡尔曼滤波器销毁 */
void shadowgate_kalman_destroy(void *filter);

// ===== 迟滞检测 =====

/** 迟滞检测器创建 */
void* shadowgate_hysteresis_create(double unlock_threshold,
                                   double lock_threshold,
                                   uint32_t unlock_samples,
                                   uint32_t lock_samples);

/** 迟滞检测器更新: 0=None, 1=Unlock, 2=Lock */
int shadowgate_hysteresis_update(void *detector, double rssi);

/** 迟滞检测器销毁 */
void shadowgate_hysteresis_destroy(void *detector);

// ===== 协议序列化 =====

/** 序列化质询请求 */
int shadowgate_create_challenge(const uint8_t *challenge,  // [in] 32 bytes
                                uint32_t sequence,
                                uint8_t *payload_out,      // [out]
                                uint32_t *payload_len);     // [in/out]

/** 解析质询响应 */
int shadowgate_parse_response(const uint8_t *data,
                              uint32_t data_len,
                              uint8_t *signature_out,     // [out] 64 bytes
                              uint32_t *sequence_out,
                              int8_t *rssi_out);

#ifdef __cplusplus
}
#endif

#endif // SHADOWGATE_CORE_H
