//! Mock Exasol engine for the `localzmq+protobuf` protocol: a `REP` peer that
//! drives a UDF client through handshake and run cycles with pre-encoded input.
//! Everything here describes observable wire behaviour per `zmqcontainer.proto`
//! and the reference C++ container; no engine internals.

pub mod payload;
mod session;
mod sink;

pub use payload::{ColumnClass, EncodedInput, FrameCursor, Int64Columns, RowSource};
pub use session::{MOCK_CONN_ID, MockError, Session};
pub use sink::{EMIT_LIMIT_BYTES, EmitCollector, EmitCounters, EmitSink};

pub trait InputSource {
    fn next_frame(&mut self) -> Option<&[u8]>;
}
