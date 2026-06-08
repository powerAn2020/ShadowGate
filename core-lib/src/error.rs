use thiserror::Error;

/// ShadowGate Core 统一错误类型
#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Cryptographic operation failed: {0}")]
    CryptoError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] bincode::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    #[error("Signature verification failed")]
    SignatureVerification,

    #[error("Challenge timeout exceeded ({0}ms)")]
    ChallengeTimeout(u64),

    #[error("Invalid state transition: {0}")]
    InvalidState(String),
}

pub type CoreResult<T> = Result<T, CoreError>;
