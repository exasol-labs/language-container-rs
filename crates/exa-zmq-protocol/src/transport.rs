use crate::error::ProtocolError;
use exa_proto::{ExascriptRequest, ExascriptResponse};
use prost::Message;

pub struct ZmqTransport {
    socket: zmq::Socket,
}

impl ZmqTransport {
    /// Connect a DEALER socket to `endpoint` (e.g. "tcp://localhost:6583").
    ///
    /// The DB binds a ROUTER socket. In the DEALER→ROUTER pattern the DEALER
    /// sends one raw payload frame; the ROUTER prepends the sender identity
    /// for its own routing and delivers [identity, payload] to the DB
    /// application. The DB replies [identity, payload]; ROUTER strips the
    /// identity before delivering to the DEALER, so both send and recv see
    /// exactly one frame — the prost-encoded message. A REQ socket would
    /// auto-insert an empty delimiter frame, making the DB see a three-frame
    /// envelope [id, empty, payload]; it cannot parse the protobuf from the
    /// empty frame and never replies, causing the post-MT_CLIENT hang.
    pub fn connect(endpoint: &str) -> Result<Self, ProtocolError> {
        let ctx = zmq::Context::new();
        let socket = ctx.socket(zmq::DEALER)?;
        socket.set_linger(0)?;
        socket.connect(endpoint)?;
        Ok(ZmqTransport { socket })
    }

    /// Send one prost-encoded message to the DB's ROUTER socket.
    ///
    /// DEALER→ROUTER requires an empty delimiter frame before the payload so
    /// the ROUTER can locate the boundary between the routing envelope and the
    /// message body. Frame layout on the wire: [empty][protobuf_bytes].
    pub fn send(&self, req: &ExascriptRequest) -> Result<(), ProtocolError> {
        let buf = req.encode_to_vec();
        tracing::debug!(mt = req.r#type, len = buf.len(), "send");
        self.socket.send(b"" as &[u8], zmq::SNDMORE)?;
        self.socket.send(&buf, 0)?;
        Ok(())
    }

    /// Receive one message from the DB's ROUTER socket and decode it.
    ///
    /// The ROUTER prepends an empty delimiter frame before the payload (mirror
    /// of what `send` writes). We discard that frame and decode the second one.
    pub fn recv(&self) -> Result<ExascriptResponse, ProtocolError> {
        tracing::debug!("recv: waiting");
        let _ = self.socket.recv_bytes(0)?; // empty delimiter frame
        let bytes = self.socket.recv_bytes(0)?;
        tracing::debug!(
            len = bytes.len(),
            more = self.socket.get_rcvmore().unwrap_or(false),
            "recv: got frame"
        );
        let resp = ExascriptResponse::decode(bytes.as_slice())?;
        tracing::debug!(mt = resp.r#type, "recv: decoded");
        Ok(resp)
    }
}
