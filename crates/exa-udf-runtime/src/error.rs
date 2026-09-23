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

impl RuntimeError {
    /// Append an error the hook's context recorded to a hook's `Udf` error,
    /// unless the hook text already contains it. This is the one place that
    /// decides how a hook failure absorbs its context's recorded detail, so
    /// the cleanup hook and the single-call hooks report it the same way.
    pub(crate) fn with_recorded_detail(self, detail: Option<String>) -> RuntimeError {
        match (self, detail) {
            (RuntimeError::Udf(text), Some(detail)) if !text.contains(&detail) => {
                RuntimeError::Udf(format!("{text}: {detail}"))
            }
            (error, _) => error,
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
