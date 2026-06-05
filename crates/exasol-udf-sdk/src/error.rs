use thiserror::Error;

#[derive(Debug, Error)]
pub enum UdfError {
    #[error("{0}")]
    User(String),
    #[error("Feature not supported in v1: {0}")]
    Unimplemented(String),
    #[error("Type error: {0}")]
    Type(String),
}
