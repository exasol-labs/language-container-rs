use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use crate::rowset::{EmitBuffer, HostContextBridge, InputRowSet};
use exa_zmq_protocol::{HostAction, HostEvent, Protocol, UdfMeta, ZmqTransport};
use exasol_udf_sdk::context::UdfContext;

/// Drive the run phase: feed input batches into the UDF and flush its output
/// until the DB signals `MT_DONE`.
///
/// The DB binds a REP socket, so every wire exchange is strictly
/// client-send-then-receive. The client opens the phase by sending `MT_RUN`,
/// then pulls each input batch with `MT_NEXT`, and finally sends `MT_DONE`.
/// The DB's reply to each request classifies what to do next.
///
/// Emit is streamed: the bridge flushes a mid-run `MT_EMIT` each time the
/// buffer crosses 4,000,000 bytes, then `consume_input` sends a tail flush for
/// any residual rows after the UDF's `run` returns.
///
/// Scalar (`ExactlyOnce`) and set (`Multiple`) UDFs share the same loop: the DB
/// controls batch sizing on the wire, and the unified `HostContextBridge`
/// presents each batch through the canonical `while ctx.next()?` iteration, so
/// the dispatcher does not need to special-case the iteration shape.
pub fn run_udf(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    meta: &UdfMeta,
) -> Result<(), RuntimeError> {
    loop {
        match request(transport, proto, proto.run_request())? {
            HostEvent::Run => {}
            // The DB ends the session by answering MT_RUN with MT_CLEANUP.
            HostEvent::Cleanup => break,
            HostEvent::Close(msg) => return close_error(msg),
            _ => {}
        }

        if let Some(early) = consume_input(transport, proto, udf, meta)? {
            return early;
        }

        match request(transport, proto, proto.done_request())? {
            HostEvent::Done => {}
            HostEvent::Cleanup => break,
            HostEvent::Close(msg) => return close_error(msg),
            _ => {}
        }
    }

    // Client-initiated teardown: MT_FINISHED, then the DB echoes it.
    request(transport, proto, proto.finished_reply())?;
    Ok(())
}

/// Pull and process input batches with MT_NEXT until the DB answers with
/// MT_DONE (input exhausted for this run). Returns `Some(result)` if the DB
/// closed or cleaned up mid-stream so the caller can short-circuit.
fn consume_input(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    meta: &UdfMeta,
) -> Result<Option<Result<(), RuntimeError>>, RuntimeError> {
    loop {
        match request(transport, proto, proto.next_request())? {
            HostEvent::NextData(table) => {
                let emitted = run_batch(transport, proto, udf, &table, meta)?;
                // Tail flush: `run_batch` flushes mid-run whenever the emit
                // buffer crosses its byte threshold, so what returns here is
                // only the residual rows accumulated since the last flush (or
                // the whole batch if it never crossed the threshold). An empty
                // residual means every row was already flushed mid-run.
                if !emitted.is_empty() {
                    let out = emitted.to_proto(&meta.output_columns);
                    request(transport, proto, proto.emit_request(out))?;
                }
            }
            // MT_DONE in answer to MT_NEXT means input is exhausted.
            HostEvent::Done => return Ok(None),
            HostEvent::TryAgain | HostEvent::Reset => continue,
            HostEvent::Cleanup => return Ok(Some(Ok(()))),
            HostEvent::Close(msg) => return Ok(Some(close_error(msg))),
            HostEvent::Run
            | HostEvent::EmitAck
            | HostEvent::Finished
            | HostEvent::Meta(_)
            | HostEvent::Pending
            | HostEvent::Ping(_)
            | HostEvent::SingleCall { .. }
            | HostEvent::SingleCallAck => {}
            HostEvent::ConnInfo(_) => {}
        }
    }
}

fn close_error(msg: Option<String>) -> Result<(), RuntimeError> {
    Err(RuntimeError::Udf(
        msg.unwrap_or_else(|| "connection closed by database".into()),
    ))
}

/// Send one request and return the classified response event, replying to a
/// ping transparently and retrying the same request once if the DB pings mid
/// exchange (REQ stays in lockstep: a ping reply is itself a request/reply).
fn request(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    req: exa_proto::ExascriptRequest,
) -> Result<HostEvent, RuntimeError> {
    transport.send(&req)?;
    let resp = transport.recv()?;
    let (event, action) = proto.step(resp)?;
    if let Some(HostAction::PingReply(s)) = action {
        return request(transport, proto, proto.ping_reply(&s));
    }
    Ok(event)
}

/// Materialise one input batch, run the UDF over it, and return its emit buffer.
///
/// The bridge and its borrows are confined to this function so the raw
/// double-indirection context pointer cannot outlive the live references.
///
/// `transport` and `proto` are shared (via a single `RefCell`) between two
/// closures given to the bridge: the mid-run emit flusher (always present),
/// which sends `MT_EMIT` whenever the buffer crosses the byte threshold; and,
/// when the `connect-back` feature is enabled, the credential fetcher, which
/// sends `MT_IMPORT` each time the UDF calls `connection(name)`. The two
/// closures never overlap because UDF execution is single-threaded and the
/// outer dispatch loop is blocked here.
fn run_batch(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    table: &exa_proto::ExascriptTableData,
    meta: &UdfMeta,
) -> Result<EmitBuffer, RuntimeError> {
    let mut input = InputRowSet::from_proto(table, &meta.input_columns);
    let mut emit_buf = EmitBuffer::new();
    {
        // Both the emit flusher and (when enabled) the credential fetcher need
        // `&mut Protocol` for wire I/O, yet they are distinct closures captured
        // by the same bridge. A single `RefCell<&mut Protocol>` — created once
        // and shared by reference — reconciles this: each closure borrows the
        // cell mutably only for one send/recv exchange. The borrows never
        // overlap because calls are serial: the dispatch loop is blocked here
        // and the UDF runs single-threaded, so one closure always runs to
        // completion before another is entered. Two cells over the same `&mut`
        // would be unsound, hence exactly one.
        let proto_cell = std::cell::RefCell::new(proto);
        let cell_ref = &proto_cell;

        // Mid-run flusher: send one pre-built proto table as MT_EMIT. The
        // serialise + clear step happens in the bridge's `emit()` (row path) or
        // in `push_batch` (batch path) before the flusher is called, so the
        // flusher only needs to send the table and await the ack. A zero-row
        // table is a no-op (no zero-row MT_EMIT on the wire).
        let flusher: crate::rowset::EmitFlusher = Box::new(
            move |table: exa_proto::ExascriptTableData| -> Result<(), exasol_udf_sdk::error::UdfError> {
                if table.rows == 0 {
                    return Ok(());
                }
                let mut proto = cell_ref.borrow_mut();
                let req = proto.emit_request(table);
                request(transport, &mut proto, req)
                    .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
                Ok(())
            },
        );

        #[cfg(feature = "connect-back")]
        let conn_requester: crate::rowset::ConnRequester = Box::new(move |conn_name: &str| {
            let mut proto = cell_ref.borrow_mut();
            let req = proto.import_connection_request(conn_name);
            transport
                .send(&req)
                .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
            let resp = transport
                .recv()
                .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
            let (event, _) = proto
                .step(resp)
                .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
            match event {
                exa_zmq_protocol::HostEvent::ConnInfo(ci) => Ok(ci),
                _ => Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "MT_IMPORT reply was not ConnInfo".into(),
                )),
            }
        });

        let mut bridge = HostContextBridge::new(
            &mut input,
            &mut emit_buf,
            &meta.input_columns,
            &meta.output_columns,
            flusher,
            crate::rowset::HandshakeMeta::from(meta),
            #[cfg(feature = "connect-back")]
            conn_requester,
        );
        // ABI contract: pass a pointer to a `&mut dyn UdfContext` (double
        // indirection). The run shim restores it via
        // `&mut *(ctx as *mut &mut dyn UdfContext)`.
        let mut dyn_ref: &mut dyn UdfContext = &mut bridge;
        let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;
        let mut error_ptr: *mut std::ffi::c_char = std::ptr::null_mut();
        let rc = unsafe { udf.run(ctx_ptr, &mut error_ptr as *mut *mut std::ffi::c_char) };
        if rc != 0 {
            let extra = if !error_ptr.is_null() {
                Some(unsafe { crate::single_call::take_c_string(error_ptr) })
            } else {
                None
            };
            let msg = match extra {
                Some(e) => format!("UDF run returned error code {rc}: {e}"),
                None => format!("UDF run returned error code {rc}"),
            };
            return Err(RuntimeError::Udf(msg));
        }
    }
    Ok(emit_buf)
}
