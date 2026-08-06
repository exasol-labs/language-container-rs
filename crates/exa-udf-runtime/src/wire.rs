//! The single lockstep DB exchange shared by both dispatchers, so neither can
//! drift from the other's wire policy.
use crate::error::RuntimeError;
use exa_zmq_protocol::{HostAction, HostEvent, Protocol, ZmqTransport};

/// Send one request and return the classified response event.
///
/// A ping mid exchange is answered transparently: the ping reply is itself a
/// request/reply, so REQ stays in lockstep and whatever the DB answers it with
/// becomes the outcome of the original request. Loops rather than recurses, so
/// an arbitrarily long ping run stays flat.
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

/// Build an on-demand `MT_IMPORT` credential fetcher: given a CONNECTION name,
/// sends `MT_IMPORT` and returns the resulting `ConnInfo`.
///
/// Borrows `proto_cell` mutably only for the one exchange, so it never overlaps
/// the caller's own borrows of the same cell.
#[cfg(feature = "connect-back")]
pub(crate) fn conn_requester<'a>(
    transport: &'a ZmqTransport,
    proto_cell: &'a std::cell::RefCell<&'a mut Protocol>,
) -> crate::rowset::ConnRequester<'a> {
    Box::new(move |conn_name: &str| {
        let mut proto = proto_cell.borrow_mut();
        let req = proto.import_connection_request(conn_name);
        let event = request(transport, &mut proto, req)
            .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
        match event {
            HostEvent::ConnInfo(ci) => Ok(ci),
            HostEvent::Close(msg) => Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                msg.unwrap_or_else(|| "connection closed by database".into()),
            )),
            _ => Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "MT_IMPORT reply was not ConnInfo".into(),
            )),
        }
    })
}
