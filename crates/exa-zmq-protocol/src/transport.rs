use crate::error::ProtocolError;
use exa_proto::{ExascriptRequest, ExascriptResponse};
use prost::Message;

/// Poll interval for blocking recv/send, in milliseconds. Matches the reference
/// libexaudflib client's `RCVTIMEO`/`SNDTIMEO`. Kept short on purpose: it is a
/// *poll* interval, not a deadline — each expiry returns `EAGAIN`, which the
/// retry loop treats as "still waiting" rather than fatal, so the loop stays
/// responsive (and could log progress) instead of blocking opaquely forever.
const POLL_INTERVAL_MS: i32 = 1000;

pub struct ZmqTransport {
    socket: zmq::Socket,
}

/// True when a ZMQ error is the transient `EAGAIN`/timeout (the `RCVTIMEO` or
/// `SNDTIMEO` poll interval elapsed with no message) as opposed to a genuine
/// socket failure. Only `EAGAIN` is retryable; everything else is fatal.
fn is_transient_timeout(err: &zmq::Error) -> bool {
    matches!(err, zmq::Error::EAGAIN)
}

/// Retries a single blocking socket operation through transient `EAGAIN`
/// timeouts until it succeeds or a genuine socket error occurs. No wall-clock
/// cap: the database's own session watchdog ends a genuinely wedged peer, so
/// a client-side deadline would only turn a slow-but-alive reply into a crash.
/// Socket-free so it is unit-testable without a real `zmq::Socket`.
fn retry_transient<T>(
    mut op: impl FnMut() -> Result<T, zmq::Error>,
    what: &str,
) -> Result<T, ProtocolError> {
    loop {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) if is_transient_timeout(&err) => {
                tracing::debug!("{what}: transient EAGAIN timeout, still waiting");
            }
            Err(err) => return Err(err.into()),
        }
    }
}

impl ZmqTransport {
    /// Connect a REQ socket to `endpoint` (e.g. "tcp://localhost:6583").
    ///
    /// The DB binds a REP socket. REQ↔REP enforces strict lock-step
    /// alternation: the client sends exactly one request, waits for exactly
    /// one reply, then may send again. The REQ socket manages the empty
    /// delimiter frame automatically; both sides deliver and receive a single
    /// payload frame — the prost-encoded message.
    pub fn connect(endpoint: &str) -> Result<Self, ProtocolError> {
        let ctx = zmq::Context::new();
        let socket = ctx.socket(zmq::REQ)?;
        // Match reference libexaudflib socket options (from script-languages-release source).
        // LINGER=0: discard pending messages on close — IPC channel, DB manages its own teardown.
        //   Safe for the success-path `process::exit(0)`: the final MT_FINISHED is acked by the DB
        //   (its reply is recv'd) before exit, so there is no pending outbound message to linger for.
        // RCVTIMEO/SNDTIMEO: a *poll* interval, not a deadline. `send`/`recv` retry on the resulting
        //   `EAGAIN` (see `send`/`recv`) so a slow-but-alive DB does not abort the UDF.
        socket.set_linger(0)?;
        socket.set_rcvtimeo(POLL_INTERVAL_MS)?;
        socket.set_sndtimeo(POLL_INTERVAL_MS)?;
        socket.connect(endpoint)?;
        Ok(ZmqTransport { socket })
    }

    /// Encodes and delivers the single request frame; the REQ lock-step
    /// contract ensures the DB's REP socket is in receive state.
    ///
    /// Under backpressure the `SNDTIMEO` poll interval can elapse before ZMQ
    /// queues the frame, returning `EAGAIN`. Because the REQ socket has not yet
    /// accepted the message, re-sending the frame is safe and preserves
    /// lockstep, so we retry transient timeouts rather than treating them as
    /// fatal — re-encoding the request on each retry, since `Socket::send`
    /// consumes the message. The frame is queued at most once: the first
    /// non-`EAGAIN` return (success or genuine error) ends the loop.
    ///
    /// Re-encoding on retry is deliberate: `Socket::send` consumes and frees
    /// the `Message` on failure, and the zero-copy hand-off to libzmq is worth
    /// more than a retained buffer. Send-side `EAGAIN` only occurs before the
    /// peer has connected (the ~100-byte `MT_CLIENT` handshake) or after the
    /// peer is gone; the REQ pipe's high-water mark counts messages and
    /// lockstep keeps one in flight, so a large `MT_EMIT` never re-encodes in
    /// practice.
    pub fn send(&self, req: &ExascriptRequest) -> Result<(), ProtocolError> {
        tracing::debug!(mt = req.r#type, len = req.encoded_len(), "send");
        retry_transient(
            || self.socket.send(zmq::Message::from(req.encode_to_vec()), 0),
            "send",
        )
    }

    /// Blocks until the DB's REP socket delivers its single reply frame; the
    /// REQ lock-step contract guarantees this is the only frame.
    ///
    /// A reply slower than the `RCVTIMEO` poll interval returns `EAGAIN`; the
    /// reply has not arrived, so re-receiving on the same socket is correct and
    /// keeps lockstep (no new request is sent — the pending reply is still
    /// awaited). We retry transient timeouts so a slow-but-alive DB does not
    /// abort the VM.
    pub fn recv(&self) -> Result<ExascriptResponse, ProtocolError> {
        tracing::debug!("recv: waiting");
        let msg = retry_transient(|| self.socket.recv_msg(0), "recv")?;
        tracing::debug!(len = msg.len(), "recv: got frame");
        let resp = ExascriptResponse::decode(&*msg)?;
        tracing::debug!(mt = resp.r#type, "recv: decoded");
        Ok(resp)
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
