//! 非对称密钥生成、签名、验证模块
//! 依赖 ed25519-dalek 实现 Ed25519 签名方案
//!
//! 协议流程:
//! 1. 配对手持设备生成密钥对
//! 2. 交换公钥建立信任
//! 3. PC 发送随机 Challenge
//! 4. Android 用私钥签名后返回
//! 5. PC 用 Android 公钥验签

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// 密钥对 (公钥 + 私钥)
#[derive(Clone)]
pub struct KeyPair {
    pub public_key: PublicKey,
    pub secret_key: SecretKey,
}

/// 公钥 (可安全分发)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicKey {
    /// 32 字节 Ed25519 公钥
    pub bytes: [u8; 32],
}

/// 私钥 (绝不可分发)
#[derive(Clone)]
pub struct SecretKey {
    inner: SigningKey,
}

/// 签名数据
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignatureBytes {
    pub bytes: [u8; 64],
}

impl KeyPair {
    /// 生成新的 Ed25519 密钥对
    pub fn generate() -> CoreResult<Self> {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();

        Ok(KeyPair {
            public_key: PublicKey {
                bytes: verifying_key.to_bytes(),
            },
            secret_key: SecretKey {
                inner: signing_key,
            },
        })
    }

    /// 从字节数组恢复密钥对 (用于从 Keystore 加载)
    pub fn from_bytes(seed: &[u8; 32]) -> CoreResult<Self> {
        let signing_key = SigningKey::from_bytes(seed);
        let verifying_key = signing_key.verifying_key();

        Ok(KeyPair {
            public_key: PublicKey {
                bytes: verifying_key.to_bytes(),
            },
            secret_key: SecretKey {
                inner: signing_key,
            },
        })
    }

    /// 对消息签名 (用于 Challenge-Response)
    pub fn sign(&self, message: &[u8]) -> SignatureBytes {
        let signature: Signature = self.secret_key.inner.sign(message);
        SignatureBytes {
            bytes: signature.to_bytes(),
        }
    }

    /// 导出公钥字节
    pub fn public_key_bytes(&self) -> &[u8; 32] {
        &self.public_key.bytes
    }

    /// 导出私钥种子 (谨慎使用，仅用于 Keystore 存储)
    pub fn secret_seed(&self) -> [u8; 32] {
        self.secret_key.inner.to_bytes()
    }
}

impl PublicKey {
    /// 从 32 字节数组构造公钥
    pub fn from_bytes(bytes: [u8; 32]) -> CoreResult<Self> {
        // 验证字节是否为有效 Ed25519 公钥
        VerifyingKey::from_bytes(&bytes)
            .map_err(|e| CoreError::InvalidKey(format!("invalid public key: {}", e)))?;
        Ok(PublicKey { bytes })
    }

    /// 验证签名
    pub fn verify(&self, message: &[u8], signature: &SignatureBytes) -> CoreResult<bool> {
        let verifying_key = VerifyingKey::from_bytes(&self.bytes)
            .map_err(|e| CoreError::InvalidKey(format!("invalid public key: {}", e)))?;
        let sig = Signature::from_bytes(&signature.bytes);
        Ok(verifying_key.verify(message, &sig).is_ok())
    }

    /// 序列化为 Base64 字符串 (用于 QR 码 / 配置存储)
    pub fn to_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.bytes)
    }

    /// 从 Base64 字符串反序列化
    pub fn from_base64(encoded: &str) -> CoreResult<Self> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| CoreError::InvalidKey(format!("base64 decode failed: {}", e)))?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| CoreError::InvalidKey("expected 32 bytes".into()))?;
        Self::from_bytes(arr)
    }
}

impl core::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KeyPair")
            .field("public_key", &self.public_key)
            .field("secret_key", &"[REDACTED]")
            .finish()
    }
}

impl core::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SecretKey([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let kp = KeyPair::generate().unwrap();
        assert_eq!(kp.public_key_bytes().len(), 32);
    }

    #[test]
    fn test_sign_and_verify() {
        let kp = KeyPair::generate().unwrap();
        let message = b"hello shadowgate challenge";
        let sig = kp.sign(message);
        let valid = kp.public_key.verify(message, &sig).unwrap();
        assert!(valid);

        // 篡改消息应验证失败
        let valid = kp.public_key.verify(b"tampered message", &sig).unwrap();
        assert!(!valid);
    }

    #[test]
    fn test_public_key_serialization() {
        let kp = KeyPair::generate().unwrap();
        let encoded = kp.public_key.to_base64();
        let decoded = PublicKey::from_base64(&encoded).unwrap();
        assert_eq!(kp.public_key.bytes, decoded.bytes);
    }
}
