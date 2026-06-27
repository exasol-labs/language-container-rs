use crate::error::ProtocolError;
use exa_proto::{ExascriptRequest, ExascriptResponse};
use prost::Message;
use std::time::{Duration, Instant};

/// Poll interval for blocking recv/send, in milliseconds. Matches the reference
/// libexaudflib client's `RCVTIMEO`/`SNDTIMEO`. Kept short on purpose: it is a
/// *poll* interval, not a deadline — each expiry returns `EAGAIN`, which the
/// retry loop treats as "still waiting" rather than fatal, so the loop stays
/// responsive (and could log progress) instead of blocking opaquely forever.
const POLL_INTERVAL_MS: i32 = 1000;

/// Generous overall wall-clock cap on a single blocking recv/send before the
/// transport gives up and surfaces a timeout error.
///
/// The 1 s `RCVTIMEO` alone is *not* a safe deadline: under a loaded cluster the
/// engine routinely takes longer than 1 s to reply (e.g. while draining a large
/// MT_EMIT stream from many concurrent VMs). The reference client survives this
/// by retrying on `EAGAIN` and polling effectively unbounded — relying on the
/// engine's own session watchdog to kill genuinely stalled sessions. This port
/// dropped the retry, so a single slow reply aborted the VM (crash signature
/// `handleDead() ... state=15; signaled=FALSE`, then the engine SIGKILLs the
/// sibling VMs).
///
/// We restore the retry but keep a backstop far above any plausible under-load
/// reply latency, so it never trips in practice yet still bounds a truly wedged
/// peer should the engine watchdog ever fail to act.
const MAX_TOTAL_WAIT: Duration = Duration::from_secs(120);

pub struct ZmqTransport {
    socket: zmq::Socket,
}

/// True when a ZMQ error is the transient `EAGAIN`/timeout (the `RCVTIMEO` or
/// `SNDTIMEO` poll interval elapsed with no message) as opposed to a genuine
/// socket failure. Only `EAGAIN` is retryable; everything else is fatal.
fn is_transient_timeout(err: &zmq::Error) -> bool {
    matches!(err, zmq::Error::EAGAIN)
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
    /// accepted the message, re-sending the *same* frame is safe and preserves
    /// lockstep, so we retry transient timeouts rather than treating them as
    /// fatal. The frame is queued at most once: the first non-`EAGAIN` return
    /// (success or genuine error) ends the loop.
    pub fn send(&self, req: &ExascriptRequest) -> Result<(), ProtocolError> {
        let buf = req.encode_to_vec();
        tracing::debug!(mt = req.r#type, len = buf.len(), "send");
        self.retry_transient(|| self.socket.send(&buf, 0), "send")
    }

    /// Blocks until the DB's REP socket delivers its single reply frame; the
    /// REQ lock-step contract guarantees this is the only frame.
    ///
    /// A reply slower than the `RCVTIMEO` poll interval returns `EAGAIN`; the
    /// reply has not arrived, so re-receiving on the same socket is correct and
    /// keeps lockstep (no new request is sent — the pending reply is still
    /// awaited). We retry transient timeouts so a slow-but-alive DB does not
    /// abort the VM. See `MAX_TOTAL_WAIT` for why an effectively-unbounded poll
    /// is the right default and what backstops it.
    pub fn recv(&self) -> Result<ExascriptResponse, ProtocolError> {
        tracing::debug!("recv: waiting");
        let bytes = self.retry_transient(|| self.socket.recv_bytes(0), "recv")?;
        tracing::debug!(len = bytes.len(), "recv: got frame");
        let resp = ExascriptResponse::decode(bytes.as_slice())?;
        tracing::debug!(mt = resp.r#type, "recv: decoded");
        Ok(resp)
    }

    /// Run a single blocking socket operation, polling through transient
    /// `EAGAIN` timeouts until it completes, a genuine socket error occurs, or
    /// `MAX_TOTAL_WAIT` elapses. Genuine errors propagate immediately.
    fn retry_transient<T>(
        &self,
        mut op: impl FnMut() -> Result<T, zmq::Error>,
        what: &str,
    ) -> Result<T, ProtocolError> {
        let start = Instant::now();
        loop {
            match op() {
                Ok(value) => return Ok(value),
                Err(err) if is_transient_timeout(&err) => {
                    let elapsed = start.elapsed();
                    if elapsed >= MAX_TOTAL_WAIT {
                        return Err(ProtocolError::Protocol(format!(
                            "{what} timed out after {elapsed:?} of polling a non-responsive \
                             database (poll interval {POLL_INTERVAL_MS} ms)"
                        )));
                    }
                    tracing::debug!(?elapsed, "{what}: transient EAGAIN timeout, still waiting");
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}
