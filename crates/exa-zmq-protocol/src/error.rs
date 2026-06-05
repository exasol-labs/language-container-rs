use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("ZMQ error: {0}")]
    Zmq(#[from] zmq::Error),
    #[error("Prost decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("Unexpected message type {0:?} in state {1}")]
    UnexpectedMessage(i32, &'static str),
    #[error("Protocol error: {0}")]
    Protocol(String),
}
