use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Loader error: {0}")]
    Loader(String),
    #[error("ABI version mismatch: expected {expected}, found {found}")]
    AbiMismatch { expected: u32, found: u32 },
    #[error("Fingerprint mismatch: expected {expected}, found {found}")]
    FingerprintMismatch { expected: String, found: String },
    #[error("Unsupported feature: {0}")]
    Unsupported(String),
    #[error("Protocol error: {0}")]
    Protocol(#[from] exa_zmq_protocol::ProtocolError),
    #[error("UDF error: {0}")]
    Udf(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<libloading::Error> for RuntimeError {
    fn from(e: libloading::Error) -> Self {
        RuntimeError::Loader(e.to_string())
    }
}
