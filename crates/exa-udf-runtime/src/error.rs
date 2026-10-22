use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Loader error: {0}")]
    Loader(String),
    #[error("ABI version mismatch: expected {expected}, found {found}")]
    AbiMismatch { expected: u32, found: u32 },
    #[error("Fingerprint mismatch: expected {expected}, found {found}")]
    FingerprintMismatch { expected: String, found: String },
    #[error("Output shape mismatch: UDF compiled as {compiled} but registered as {registered}")]
    OutputShapeMismatch {
        compiled: &'static str,
        registered: &'static str,
    },
    #[error("Unsupported feature: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Protocol(#[from] exa_zmq_protocol::ProtocolError),
    #[error("UDF error: {0}")]
    Udf(String),
    /// A hook that failed without text of its own: it panicked or wrote none.
    #[error("UDF error: {0}")]
    Unexplained(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<libloading::Error> for RuntimeError {
    fn from(e: libloading::Error) -> Self {
        RuntimeError::Loader(e.to_string())
    }
}

impl RuntimeError {
    /// Explain a hook failure that carries no text of its own with the
    /// context's recorded error; a hook's own text is the author's final word.
    pub(crate) fn with_recorded_detail(self, detail: Option<String>) -> RuntimeError {
        match (self, detail) {
            (RuntimeError::Unexplained(text), Some(detail)) => {
                RuntimeError::Udf(format!("{text}: {detail}"))
            }
            (error, _) => error,
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
