mod artifact;
#[cfg(feature = "connect-back")]
mod connect_back;
mod dispatch;
mod error;
mod loader;
mod rowset;
mod schema_check;
mod single_call;

pub use artifact::{parse_debug_level, parse_udf_object_path};
pub use error::RuntimeError;
pub use loader::LoadedUdf;
pub use rowset::{EmitBuffer, HandshakeMeta, HostContextBridge, InputRowSet};

use exa_zmq_protocol::{HostAction, HostEvent, Protocol, ProtocolError, UdfMeta, ZmqTransport};

/// UDF runtime error close code surfaced to the DB in the `F-UDF-CL-RUST-####`
/// close message.
const UDF_ERROR_CLOSE_CODE: u32 = 9001;

/// Close code surfaced when the UDF's annotated schema does not match the
/// column metadata the database sent during the handshake.
const SCHEMA_MISMATCH_CLOSE_CODE: u32 = 1001;

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
    ///
    /// `on_level_resolved` is called once immediately after the handshake, with
    /// the log level parsed from the script's `%udf_debug_level` directive (or
    /// `INFO` when absent). The caller uses this to adjust the subscriber's
    /// effective filter — typically by modifying a `tracing_subscriber::reload`
    /// handle that was installed in `main()`. Calling `reload::Handle::modify`
    /// internally invokes `rebuild_interest_cache()`, which updates the
    /// process-global `MAX_LEVEL` atomic that `LevelFilter::current()` reads —
    /// so `ctx.debug_level()` (which reads `current()`) observes the resolved
    /// level after the hook returns.
    pub fn run(&self, on_level_resolved: impl Fn(tracing::Level)) -> Result<(), RuntimeError> {
        let transport = ZmqTransport::connect(&self.endpoint)?;
        let mut proto = Protocol::new();

        transport.send(&proto.client_request(&self.client_name))?;

        let meta = self.handshake(&transport, &mut proto)?;

        // Parse %udf_debug_level from the script source and apply it via the
        // caller-supplied hook. This must happen after the handshake delivers
        // source_code; lines emitted before this point use the initial INFO level.
        on_level_resolved(parse_debug_level(&meta.source_code));

        // Root span: tag every subsequent log line with the VM's identity so
        // multi-node output can be de-interleaved. pid is always present;
        // node_id/session_id/vm_id come from the handshake metadata.
        // vm_id is 0 when the DB did not supply one (single-node deployments).
        // ERROR level ensures the span is entered and its fields appear in the
        // output even when the resolved level is WARN or ERROR — an INFO-level
        // span would be filtered out and its tags lost at those settings.
        let _root = tracing::error_span!(
            "udf",
            pid = std::process::id(),
            session_id = meta.session_id(),
            node_id = meta.node_id(),
            vm_id = meta.vm_id(),
        )
        .entered();

        tracing::debug!(
            source_len = meta.source_code.len(),
            input_cols = meta.input_columns.len(),
            output_cols = meta.output_columns.len(),
            "handshake complete"
        );

        let so_path = match parse_udf_object_path(&meta.source_code) {
            Some(p) => p,
            None => {
                let e = RuntimeError::Unsupported(
                    "no %udf_object directive in source; JIT not supported in v1".into(),
                );
                let _ = transport
                    .send(&proto.error_close_request(UDF_ERROR_CLOSE_CODE, &e.to_string()));
                return Err(e);
            }
        };
        tracing::debug!(?so_path, "resolved udf object");

        let udf = match LoadedUdf::open(&so_path, &meta.script_name) {
            Ok(u) => u,
            Err(e) => {
                let _ = transport
                    .send(&proto.error_close_request(UDF_ERROR_CLOSE_CODE, &e.to_string()));
                return Err(e);
            }
        };
        tracing::debug!("udf loaded; entering run loop");

        // Validate the UDF's annotated schema (if any) against the metadata the
        // DB sent before doing any work. A mismatch closes the session with a
        // dedicated F-UDF-CL-RUST-#### code so the user sees the exact column
        // discrepancy rather than a runtime decode failure mid-stream.
        if let Err(e) = schema_check::validate_schema(&udf, &meta) {
            let req = proto.error_close_request(SCHEMA_MISMATCH_CLOSE_CODE, &e.to_string());
            let _ = transport.send(&req);
            unsafe { udf.destroy() };
            return Err(e);
        }

        let result = if meta.single_call_mode {
            single_call::run_single_call(&transport, &mut proto, &udf, &meta)
        } else {
            dispatch::run_udf(&transport, &mut proto, &udf, &meta)
        };
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
        // Connect-back credentials are resolved on demand per CONNECTION name via
        // the live MT_IMPORT exchange, not buffered onto the handshake meta. A
        // `HostEvent::ConnInfo` arriving here is simply ignored.
        loop {
            let resp = transport.recv()?;
            let (event, action) = proto.step(resp)?;
            match action {
                Some(HostAction::PingReply(s)) => transport.send(&proto.ping_reply(&s))?,
                Some(HostAction::MetaRequest) => transport.send(&proto.meta_request())?,
                _ => {}
            }
            match event {
                HostEvent::Meta(m) => {
                    return Ok(m);
                }
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
