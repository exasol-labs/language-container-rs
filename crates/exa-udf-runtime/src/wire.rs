//! The single lockstep DB exchange shared by both dispatchers.
//!
//! Both `dispatch::run_udf` (streaming SCALAR/SET) and
//! `single_call::run_single_call` talk to the same DB over the same REQ/REP
//! socket and must agree on two decisions: how a ping mid-exchange is
//! transparently answered without breaking REQ lockstep, and how an
//! `MT_CLOSE` event becomes a `RuntimeError`. This module is their one owner
//! so neither dispatcher can drift from the other's wire policy.
use crate::error::RuntimeError;
use exa_zmq_protocol::{HostAction, HostEvent, Protocol, ZmqTransport};

/// Send one request and return the classified response event, answering a ping
/// mid exchange transparently: the ping reply is itself a request/reply so REQ
/// stays in lockstep, and whatever the DB answers it with becomes the outcome
/// of the original request. Every ping in a row is answered this way, not just
/// the first — the DB may ping again in answer to a ping reply — and the loop
/// keeps an arbitrarily long ping run flat instead of one stack frame deep per
/// ping.
pub(crate) fn request(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    req: exa_proto::ExascriptRequest,
) -> Result<HostEvent, RuntimeError> {
    let mut req = req;
    loop {
        transport.send(&req)?;
        let resp = transport.recv()?;
        let (event, action) = proto.step(resp)?;
        match action {
            Some(HostAction::PingReply(s)) => req = proto.ping_reply(&s),
            _ => return Ok(event),
        }
    }
}

/// Map an `MT_CLOSE` event's optional message to the runtime error a
/// dispatcher returns when the DB ends the session with an exception.
pub(crate) fn close_error(msg: Option<String>) -> Result<(), RuntimeError> {
    Err(RuntimeError::Udf(
        msg.unwrap_or_else(|| "connection closed by database".into()),
    ))
}

/// Build an on-demand `MT_IMPORT` credential fetcher over a shared,
/// non-overlapping-borrow `RefCell<&mut Protocol>`: given a CONNECTION name,
/// sends `MT_IMPORT` and returns the resulting `ConnInfo`.
///
/// Callers hold `transport` and `proto_cell` for one group or one single-call
/// hook invocation; the closure borrows the cell mutably only for the
/// duration of one send/recv exchange, so it never overlaps the caller's own
/// borrows of the same cell.
#[cfg(feature = "connect-back")]
pub(crate) fn conn_requester<'a>(
    transport: &'a ZmqTransport,
    proto_cell: &'a std::cell::RefCell<&'a mut Protocol>,
) -> crate::rowset::ConnRequester<'a> {
    Box::new(move |conn_name: &str| {
        let mut proto = proto_cell.borrow_mut();
        let req = proto.import_connection_request(conn_name);
        transport
            .send(&req)
            .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
        let resp = transport
            .recv()
            .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
        let (event, _) = proto
            .step(resp)
            .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
        match event {
            HostEvent::ConnInfo(ci) => Ok(ci),
            _ => Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "MT_IMPORT reply was not ConnInfo".into(),
            )),
        }
    })
}
