//! Mock Exasol engine for the `localzmq+protobuf` wire protocol.
//!
//! The database side of a UDF session is a ZeroMQ `REP` socket that reacts to
//! whatever the client (`exaudfclient`, here the `Runtime` from
//! `exa-udf-runtime`) requests. This crate is that peer, reduced to what a
//! benchmark or a protocol test needs:
//!
//! - [`Session`]: binds the socket, answers the handshake, and drives one run
//!   cycle at a time with a timed window from the `MT_RUN` reply to the
//!   client's `MT_DONE`.
//! - [`InputSource`]: yields pre-encoded `MT_NEXT` reply frames for the current
//!   cycle; [`payload`] builds them the way the database batches input.
//! - [`EmitSink`]: observes every `MT_EMIT` the client sends;
//!   [`EmitCounters`] records count, bytes and the largest message.
//!
//! Every statement about the database here describes observable wire
//! behaviour and cites the protobuf definition (`zmqcontainer.proto`, vendored
//! in `exa-proto`) or the open-source reference C++ script-language container.
//! No engine internals.

pub mod payload;
mod session;
mod sink;

pub use payload::{ColumnClass, EncodedInput, FrameCursor, Int64Columns, RowSource};
pub use session::{MOCK_CONN_ID, MockError, Session};
pub use sink::{EmitCollector, EmitCounters, EmitSink};

/// Pre-encoded `MT_NEXT` reply frames for one run cycle (one engine input
/// vector for SCALAR, one group for SET), in the order the client pulls them.
pub trait InputSource {
    /// The next frame, or `None` once the cycle's input is exhausted, at which
    /// point the session answers the client's `MT_NEXT` with `MT_DONE`.
    fn next_frame(&mut self) -> Option<&[u8]>;
}
