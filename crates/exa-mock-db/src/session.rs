use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use exa_proto::{
    ExascriptInfo, ExascriptMetadata, ExascriptRequest, ExascriptResponse, MessageType,
};
use prost::Message;

use crate::{EmitSink, InputSource};

/// The one connection id every mock session uses.
pub const MOCK_CONN_ID: u64 = 7;

#[derive(Debug)]
pub enum MockError {
    Zmq(zmq::Error),
    Decode(prost::DecodeError),
    /// The client sent a message type the session did not expect at that point.
    Unexpected {
        expected: &'static str,
        got: i32,
    },
    /// The client ended the session with `MT_CLOSE`.
    Closed(Option<String>),
}

impl std::fmt::Display for MockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MockError::Zmq(e) => write!(f, "zmq: {e}"),
            MockError::Decode(e) => write!(f, "protobuf decode: {e}"),
            MockError::Unexpected { expected, got } => {
                write!(f, "expected {expected}, client sent message type {got}")
            }
            MockError::Closed(msg) => write!(
                f,
                "client closed the session: {}",
                msg.as_deref().unwrap_or("(no message)")
            ),
        }
    }
}

impl std::error::Error for MockError {}

impl From<zmq::Error> for MockError {
    fn from(e: zmq::Error) -> Self {
        MockError::Zmq(e)
    }
}

impl From<prost::DecodeError> for MockError {
    fn from(e: prost::DecodeError) -> Self {
        MockError::Decode(e)
    }
}

/// One mock database session: a bound `REP` socket and the pre-encoded replies
/// it hands out.
///
/// Protocol as the client sees it (every exchange is client request, then one
/// reply, per `zmqcontainer.proto`):
///
/// ```text
/// MT_CLIENT -> MT_INFO          handshake: script source and identity
/// MT_META   -> MT_META          column metadata
/// MT_RUN    -> MT_RUN           open one cycle          | timed window opens
/// MT_NEXT   -> MT_NEXT | MT_DONE input batches, then exhausted
/// MT_EMIT   -> MT_EMIT          output batch, acked at once
/// MT_DONE   -> MT_DONE          cycle closed            | timed window closes
/// MT_RUN    -> MT_CLEANUP       no more cycles
/// MT_FINISHED -> MT_FINISHED    teardown
/// ```
pub struct Session {
    _ctx: zmq::Context,
    socket: zmq::Socket,
    endpoint: String,
    ipc_path: PathBuf,
    run_reply: Vec<u8>,
    done_reply: Vec<u8>,
    emit_ack: Vec<u8>,
    cleanup_reply: Vec<u8>,
    finished_reply: Vec<u8>,
}

fn reply(mt: MessageType) -> ExascriptResponse {
    ExascriptResponse {
        r#type: mt as i32,
        connection_id: MOCK_CONN_ID,
        ..Default::default()
    }
}

/// Read the `type` field (field 1, a varint) from the front of an encoded
/// `exascript_request` without decoding the payload. prost writes fields in
/// field-number order and `type` is required, so it is always first.
fn peek_type(bytes: &[u8]) -> Option<i32> {
    if bytes.first() != Some(&0x08) {
        return None;
    }
    let mut value: u64 = 0;
    let mut shift = 0;
    for &b in &bytes[1..] {
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return i32::try_from(value).ok();
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

impl Session {
    /// Bind a `REP` socket on a short `ipc://` path under the system temp dir.
    pub fn bind(tag: &str) -> Result<Self, MockError> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let ipc_path =
            std::env::temp_dir().join(format!("exa-mock-{tag}-{}-{nanos}.ipc", std::process::id()));
        let endpoint = format!("ipc://{}", ipc_path.display());
        let ctx = zmq::Context::new();
        let socket = ctx.socket(zmq::REP)?;
        socket.bind(&endpoint)?;
        Ok(Session {
            _ctx: ctx,
            socket,
            endpoint,
            ipc_path,
            run_reply: reply(MessageType::MtRun).encode_to_vec(),
            done_reply: reply(MessageType::MtDone).encode_to_vec(),
            emit_ack: reply(MessageType::MtEmit).encode_to_vec(),
            cleanup_reply: reply(MessageType::MtCleanup).encode_to_vec(),
            finished_reply: reply(MessageType::MtFinished).encode_to_vec(),
        })
    }

    /// The `ipc://` endpoint the client connects to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn recv_request(&self) -> Result<ExascriptRequest, MockError> {
        let bytes = self.socket.recv_bytes(0)?;
        let req = ExascriptRequest::decode(bytes.as_slice())?;
        if req.r#type == MessageType::MtClose as i32 {
            return Err(MockError::Closed(
                req.close.and_then(|c| c.exception_message),
            ));
        }
        Ok(req)
    }

    fn expect(
        &self,
        req: &ExascriptRequest,
        mt: MessageType,
        name: &'static str,
    ) -> Result<(), MockError> {
        if req.r#type == mt as i32 {
            Ok(())
        } else {
            Err(MockError::Unexpected {
                expected: name,
                got: req.r#type,
            })
        }
    }

    /// Answer `MT_CLIENT` with `MT_INFO` naming the `.so` to load, then
    /// `MT_META` with `meta`. Returns once the client is about to send its
    /// first `MT_RUN`.
    pub fn handshake(
        &mut self,
        udf_object: &Path,
        script_name: &str,
        meta: ExascriptMetadata,
    ) -> Result<(), MockError> {
        let req = self.recv_request()?;
        self.expect(&req, MessageType::MtClient, "MT_CLIENT")?;
        let mut info = reply(MessageType::MtInfo);
        info.info = Some(ExascriptInfo {
            database_name: "mock".into(),
            database_version: "0".into(),
            script_name: script_name.into(),
            source_code: format!("%udf_object {}", udf_object.display()),
            script_schema: "BENCH".into(),
            node_count: 1,
            ..Default::default()
        });
        self.socket.send(info.encode_to_vec(), 0)?;

        let req = self.recv_request()?;
        self.expect(&req, MessageType::MtMeta, "MT_META")?;
        let mut m = reply(MessageType::MtMeta);
        m.meta = Some(meta);
        self.socket.send(m.encode_to_vec(), 0)?;
        Ok(())
    }

    /// Drive one run cycle: answer the pending `MT_RUN`, serve `input` until it
    /// is exhausted, ack every `MT_EMIT`, and stop the clock when the client's
    /// `MT_DONE` arrives. Emitted tables reach `sink` after the window closes.
    ///
    /// The window therefore contains the client's receive, decode, dispatch,
    /// UDF body, encode and send, plus one libzmq copy per frame on this side.
    pub fn run_cycle(
        &mut self,
        input: &mut dyn InputSource,
        sink: &mut dyn EmitSink,
    ) -> Result<Duration, MockError> {
        let req = self.recv_request()?;
        self.expect(&req, MessageType::MtRun, "MT_RUN")?;
        self.socket.send(self.run_reply.as_slice(), 0)?;
        let start = Instant::now();

        let mut emits: Vec<Vec<u8>> = Vec::new();
        let elapsed = loop {
            let bytes = self.socket.recv_bytes(0)?;
            let mt = peek_type(&bytes);
            if mt == Some(MessageType::MtNext as i32) {
                match input.next_frame() {
                    Some(frame) => self.socket.send(frame, 0)?,
                    None => self.socket.send(self.done_reply.as_slice(), 0)?,
                }
            } else if mt == Some(MessageType::MtEmit as i32) {
                self.socket.send(self.emit_ack.as_slice(), 0)?;
                emits.push(bytes);
            } else if mt == Some(MessageType::MtDone as i32) {
                break start.elapsed();
            } else {
                // Anything else is decoded in full so a close message is
                // reported with its text.
                let req = ExascriptRequest::decode(bytes.as_slice())?;
                if req.r#type == MessageType::MtClose as i32 {
                    return Err(MockError::Closed(
                        req.close.and_then(|c| c.exception_message),
                    ));
                }
                return Err(MockError::Unexpected {
                    expected: "MT_NEXT, MT_EMIT or MT_DONE",
                    got: req.r#type,
                });
            }
        };

        for frame in &emits {
            let req = ExascriptRequest::decode(frame.as_slice())?;
            if let Some(emit) = req.emit {
                sink.observe(frame.len(), &emit.table);
            }
        }
        self.socket.send(self.done_reply.as_slice(), 0)?;
        Ok(elapsed)
    }

    /// End the session: answer the client's next `MT_RUN` with `MT_CLEANUP`
    /// and echo its `MT_FINISHED`.
    pub fn finish(self) -> Result<(), MockError> {
        let req = self.recv_request()?;
        self.expect(&req, MessageType::MtRun, "MT_RUN")?;
        self.socket.send(self.cleanup_reply.as_slice(), 0)?;
        let req = self.recv_request()?;
        self.expect(&req, MessageType::MtFinished, "MT_FINISHED")?;
        self.socket.send(self.finished_reply.as_slice(), 0)?;
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.ipc_path);
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
