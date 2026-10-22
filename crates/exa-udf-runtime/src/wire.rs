//! The single lockstep DB exchange shared by both dispatchers, so neither can
//! drift from the other's wire policy.
use crate::error::RuntimeError;
use exa_zmq_protocol::{Frame, HostAction, HostEvent, Protocol, ZmqTransport};
use exasol_udf_sdk::error::UdfError;

/// Send one request and return the classified response event.
///
/// A ping mid exchange is answered transparently: the ping reply is itself a
/// request/reply, so REQ stays in lockstep and whatever the DB answers it with
/// becomes the outcome of the original request. Loops rather than recurses, so
/// an arbitrarily long ping run stays flat.
pub(crate) fn request(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    req: impl Frame,
) -> Result<HostEvent, RuntimeError> {
    transport.send(&req)?;
    loop {
        let resp = transport.recv()?;
        let (event, action) = proto.step(resp)?;
        match action {
            Some(HostAction::PingReply(s)) => transport.send(&proto.ping_reply(&s))?,
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
pub(crate) fn conn_requester<'a>(
    transport: &'a ZmqTransport,
    proto_cell: &'a std::cell::RefCell<&'a mut Protocol>,
) -> crate::rowset::ConnRequester<'a> {
    Box::new(move |conn_name: &str| {
        let failed =
            |reason: String| UdfError::Connection(format!("CONNECTION {conn_name}: {reason}"));
        let mut proto = proto_cell.borrow_mut();
        let req = proto.import_connection_request(conn_name);
        match request(transport, &mut proto, req).map_err(|e| failed(e.to_string()))? {
            // After one refusal the database answers later refused lookups with empty
            // credentials and no reason; a real CONNECTION always has a kind.
            HostEvent::ConnInfo(ci) if ci.kind.is_empty() => {
                Err(failed("refused without a reason".into()))
            }
            HostEvent::ConnInfo(ci) => Ok(ci),
            HostEvent::ImportRefused(reason) => Err(UdfError::Connection(reason)),
            HostEvent::Close(msg) => Err(failed(
                msg.unwrap_or_else(|| "connection closed by database".into()),
            )),
            _ => Err(failed("MT_IMPORT reply was not ConnInfo".into())),
        }
    })
}
