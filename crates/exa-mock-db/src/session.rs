use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use exa_proto::{
    ExascriptInfo, ExascriptMetadata, ExascriptRequest, ExascriptResponse, MessageType,
};
use prost::Message;

use crate::{EmitSink, InputSource};

pub const MOCK_CONN_ID: u64 = 7;

#[derive(Debug)]
pub enum MockError {
    Zmq(zmq::Error),
    Decode(prost::DecodeError),
    Unexpected { expected: &'static str, got: i32 },
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

/// A bound `REP` socket answering one client. The timed window of a run cycle
/// spans the `MT_RUN` reply to the client's `MT_DONE`.
pub struct Session {
    _ctx: zmq::Context,
    socket: zmq::Socket,
    endpoint: String,
    ipc_path: PathBuf,
}

fn reply(mt: MessageType) -> Vec<u8> {
    ExascriptResponse {
        r#type: mt as i32,
        connection_id: MOCK_CONN_ID,
        ..Default::default()
    }
    .encode_to_vec()
}

/// Reads the leading `type` varint of an encoded request; prost emits fields in
/// field-number order, so the required `type` (field 1) is always first.
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

fn decode(bytes: &[u8]) -> Result<ExascriptRequest, MockError> {
    let req = ExascriptRequest::decode(bytes)?;
    if req.r#type == MessageType::MtClose as i32 {
        return Err(MockError::Closed(
            req.close.and_then(|c| c.exception_message),
        ));
    }
    Ok(req)
}

fn expect(req: &ExascriptRequest, mt: MessageType, name: &'static str) -> Result<(), MockError> {
    if req.r#type == mt as i32 {
        Ok(())
    } else {
        Err(MockError::Unexpected {
            expected: name,
            got: req.r#type,
        })
    }
}

impl Session {
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
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn recv(&self, mt: MessageType, name: &'static str) -> Result<ExascriptRequest, MockError> {
        let req = decode(&self.socket.recv_bytes(0)?)?;
        expect(&req, mt, name)?;
        Ok(req)
    }

    pub fn handshake(
        &mut self,
        udf_object: &Path,
        script_name: &str,
        meta: ExascriptMetadata,
    ) -> Result<(), MockError> {
        self.recv(MessageType::MtClient, "MT_CLIENT")?;
        let info = ExascriptResponse {
            r#type: MessageType::MtInfo as i32,
            connection_id: MOCK_CONN_ID,
            info: Some(ExascriptInfo {
                database_name: "mock".into(),
                database_version: "0".into(),
                script_name: script_name.into(),
                source_code: format!("%udf_object {}", udf_object.display()),
                script_schema: "BENCH".into(),
                node_count: 1,
                ..Default::default()
            }),
            ..Default::default()
        };
        self.socket.send(info.encode_to_vec(), 0)?;
        self.recv(MessageType::MtMeta, "MT_META")?;
        let m = ExascriptResponse {
            r#type: MessageType::MtMeta as i32,
            connection_id: MOCK_CONN_ID,
            meta: Some(meta),
            ..Default::default()
        };
        self.socket.send(m.encode_to_vec(), 0)?;
        Ok(())
    }

    /// Emits are acked at once and handed to `sink` after the window closes.
    pub fn run_cycle(
        &mut self,
        input: &mut dyn InputSource,
        sink: &mut dyn EmitSink,
    ) -> Result<Duration, MockError> {
        self.recv(MessageType::MtRun, "MT_RUN")?;
        self.socket.send(reply(MessageType::MtRun), 0)?;
        let start = Instant::now();
        let done = reply(MessageType::MtDone);
        let mut emits: Vec<Vec<u8>> = Vec::new();
        let elapsed = loop {
            let bytes = self.socket.recv_bytes(0)?;
            match peek_type(&bytes).and_then(|t| MessageType::try_from(t).ok()) {
                Some(MessageType::MtNext) => match input.next_frame() {
                    Some(frame) => self.socket.send(frame, 0)?,
                    None => self.socket.send(done.as_slice(), 0)?,
                },
                Some(MessageType::MtEmit) => {
                    self.socket.send(reply(MessageType::MtEmit), 0)?;
                    emits.push(bytes);
                }
                Some(MessageType::MtDone) => break start.elapsed(),
                _ => {
                    return Err(MockError::Unexpected {
                        expected: "MT_NEXT, MT_EMIT or MT_DONE",
                        got: decode(&bytes)?.r#type,
                    });
                }
            }
        };
        for frame in &emits {
            if let Some(emit) = ExascriptRequest::decode(frame.as_slice())?.emit {
                sink.observe(frame.len(), &emit.table);
            }
        }
        self.socket.send(done, 0)?;
        Ok(elapsed)
    }

    pub fn finish(self) -> Result<(), MockError> {
        self.recv(MessageType::MtRun, "MT_RUN")?;
        self.socket.send(reply(MessageType::MtCleanup), 0)?;
        self.recv(MessageType::MtFinished, "MT_FINISHED")?;
        self.socket.send(reply(MessageType::MtFinished), 0)?;
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
