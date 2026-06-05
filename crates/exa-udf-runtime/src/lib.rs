mod artifact;
mod compiler;
mod dispatch;
mod error;
mod loader;
mod rowset;

pub use artifact::parse_udf_object_path;
pub use compiler::compile_jit;
pub use error::RuntimeError;
pub use loader::LoadedUdf;
pub use rowset::{EmitBuffer, HostContextBridge, InputRowSet};

use exa_zmq_protocol::{HostAction, HostEvent, Protocol, ProtocolError, UdfMeta, ZmqTransport};

/// UDF runtime error close code surfaced to the DB in the `F-UDF-CL-RUST-####`
/// close message.
const UDF_ERROR_CLOSE_CODE: u32 = 9001;

/// The host runtime: connects to the DB's ZMQ endpoint, performs the handshake,
/// resolves and loads the precompiled UDF `.so`, and drives dispatch.
pub struct Runtime {
    endpoint: String,
    client_name: String,
}

impl Runtime {
    pub fn new(endpoint: String, client_name: String) -> Self {
        Runtime {
            endpoint,
            client_name,
        }
    }

    /// Execute one UDF session end to end: handshake → meta → resolve artifact
    /// → load → dispatch → close. On any error the message is serialised into
    /// the protocol close path with the `F-UDF-CL-RUST-` prefix and `destroy`
    /// is invoked before returning.
    pub fn run(&self) -> Result<(), RuntimeError> {
        let transport = ZmqTransport::connect(&self.endpoint)?;
        let mut proto = Protocol::new();

        transport.send(&proto.client_request(&self.client_name))?;

        let meta = self.handshake(&transport, &mut proto)?;
        tracing::debug!(
            source_len = meta.source_code.len(),
            input_cols = meta.input_columns.len(),
            output_cols = meta.output_columns.len(),
            "handshake complete"
        );

        let so_path = parse_udf_object_path(&meta.source_code).ok_or_else(|| {
            RuntimeError::Unsupported(
                "no %udf_object directive in source; JIT not supported in v1".into(),
            )
        })?;
        tracing::debug!(?so_path, "resolved udf object");

        let udf = LoadedUdf::open(&so_path)?;
        tracing::debug!("udf loaded; entering run loop");

        let result = dispatch::run_udf(&transport, &mut proto, &udf, &meta);
        tracing::debug!(ok = result.is_ok(), "run loop finished");

        match result {
            Ok(()) => {
                unsafe { udf.destroy() };
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                let req = proto.error_close_request(UDF_ERROR_CLOSE_CODE, &msg);
                let _ = transport.send(&req);
                unsafe { udf.destroy() };
                Err(e)
            }
        }
    }

    /// Drive the handshake until the `Meta` event arrives, acking with MT_META.
    fn handshake(
        &self,
        transport: &ZmqTransport,
        proto: &mut Protocol,
    ) -> Result<UdfMeta, RuntimeError> {
        loop {
            let resp = transport.recv()?;
            let (event, action) = proto.step(resp)?;
            match action {
                Some(HostAction::PingReply(s)) => transport.send(&proto.ping_reply(&s))?,
                Some(HostAction::MetaRequest) => transport.send(&proto.meta_request())?,
                _ => {}
            }
            match event {
                HostEvent::Meta(m) => return Ok(m),
                HostEvent::Pending | HostEvent::Ping(_) => continue,
                HostEvent::Close(msg) => {
                    return Err(RuntimeError::Protocol(ProtocolError::Protocol(
                        msg.unwrap_or_else(|| "closed before meta".into()),
                    )));
                }
                _ => {}
            }
        }
    }
}
