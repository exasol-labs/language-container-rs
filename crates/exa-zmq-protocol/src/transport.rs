use crate::error::ProtocolError;
use exa_proto::{ExascriptRequest, ExascriptResponse};
use prost::Message;

pub struct ZmqTransport {
    socket: zmq::Socket,
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
        socket.set_linger(0)?;
        socket.connect(endpoint)?;
        Ok(ZmqTransport { socket })
    }

    /// Encodes and delivers the single request frame; the REQ lock-step
    /// contract ensures the DB's REP socket is in receive state.
    pub fn send(&self, req: &ExascriptRequest) -> Result<(), ProtocolError> {
        let buf = req.encode_to_vec();
        tracing::debug!(mt = req.r#type, len = buf.len(), "send");
        self.socket.send(&buf, 0)?;
        Ok(())
    }

    /// Blocks until the DB's REP socket delivers its single reply frame; the
    /// REQ lock-step contract guarantees this is the only frame.
    pub fn recv(&self) -> Result<ExascriptResponse, ProtocolError> {
        tracing::debug!("recv: waiting");
        let bytes = self.socket.recv_bytes(0)?;
        tracing::debug!(len = bytes.len(), "recv: got frame");
        let resp = ExascriptResponse::decode(bytes.as_slice())?;
        tracing::debug!(mt = resp.r#type, "recv: decoded");
        Ok(resp)
    }
}
