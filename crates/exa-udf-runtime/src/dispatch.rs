use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use crate::rowset::{EmitBuffer, HostContextBridge, InputRowSet};
use exa_zmq_protocol::{HostAction, HostEvent, IterType, Protocol, UdfMeta, ZmqTransport};
use exasol_udf_sdk::context::UdfContext;
use std::cell::{Cell, RefCell};

/// Drive the run phase: process each input group and flush the UDF's output
/// until the DB signals no more groups.
///
/// The DB binds a REP socket, so every wire exchange is strictly
/// client-send-then-receive. The client opens each group with `MT_RUN`; the DB
/// answers `MT_RUN` to open a group or `MT_CLEANUP` when none remains. Within a
/// group the client pulls input batches with `MT_NEXT` until the DB answers
/// `MT_DONE`, then sends its own `MT_DONE`.
///
/// The per-group body branches on the input iteration axis (see [`run_group`]):
/// `ExactlyOnce` (SCALAR) invokes `run()` once per input row; `Multiple` (SET)
/// invokes `run()` once per group and lets `ctx.next()` span the group's
/// batches. The emit buffer is scoped to the whole group: it flushes a mid-group
/// `MT_EMIT` each time it crosses 4,000,000 bytes and a single tail `MT_EMIT`
/// before the group's `MT_DONE`.
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

        if let Some(early) = run_group(transport, proto, udf, meta)? {
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

/// How a group's input driving ended, when not the normal group boundary.
enum GroupExit {
    /// The DB answered `MT_CLEANUP` mid-input: end the whole session cleanly.
    Session,
    /// The DB answered `MT_CLOSE` mid-input: surface the exception message.
    Closed(Option<String>),
}

/// Process one input group: fetch its `MT_NEXT` batches, drive the UDF by input
/// iteration axis, and tail-flush the group's emit buffer before returning.
///
/// Returns `Ok(None)` on a normal group boundary so the caller can send its
/// `MT_DONE`; `Ok(Some(result))` if the DB closed or cleaned up mid-input so the
/// caller short-circuits `run_udf`.
///
/// `transport` and `proto` are shared (via a single `RefCell`) among the batch
/// fetcher, the mid-group emit flusher, and — when the `connect-back` feature is
/// enabled — the credential fetcher. Each closure borrows the cell mutably only
/// for one send/recv exchange; the borrows never overlap because UDF execution
/// is single-threaded and the dispatch loop is blocked here, so exactly one cell
/// over the shared `&mut Protocol` is sound.
fn run_group(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    meta: &UdfMeta,
) -> Result<Option<Result<(), RuntimeError>>, RuntimeError> {
    let mut emit_buf = EmitBuffer::new();
    // Set by the batch fetcher when the DB ends input abnormally (mid-group
    // MT_CLEANUP / MT_CLOSE); read after the run driving completes. `Option` so
    // `Cell::take` works without a `Copy` bound.
    let exit: Cell<Option<GroupExit>> = Cell::new(None);
    let mut run_err: Option<RuntimeError> = None;

    let proto_cell = RefCell::new(proto);
    let cell_ref = &proto_cell;

    // Mid-group flusher: send one pre-built proto table as MT_EMIT. A zero-row
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

    // Batch fetcher: pull the next MT_NEXT batch. `Ok(Some)` is a batch,
    // `Ok(None)` the group boundary (MT_DONE); a mid-input MT_CLEANUP / MT_CLOSE
    // records the terminal reason in `exit` and reports the group as ended.
    let exit_ref = &exit;
    let fetch = move || -> Result<Option<exa_proto::ExascriptTableData>, exasol_udf_sdk::error::UdfError> {
        loop {
            let mut proto = cell_ref.borrow_mut();
            let req = proto.next_request();
            let event = request(transport, &mut proto, req)
                .map_err(|e| exasol_udf_sdk::error::UdfError::ConnectBack(e.to_string()))?;
            match event {
                HostEvent::NextData(table) => return Ok(Some(table)),
                HostEvent::Done => return Ok(None),
                HostEvent::TryAgain | HostEvent::Reset => continue,
                HostEvent::Cleanup => {
                    exit_ref.set(Some(GroupExit::Session));
                    return Ok(None);
                }
                HostEvent::Close(msg) => {
                    exit_ref.set(Some(GroupExit::Closed(msg)));
                    return Ok(None);
                }
                _ => continue,
            }
        }
    };

    // Confine the bridge and its borrows to this block so the group-scoped
    // `emit_buf` is free for the tail flush once the UDF driving is done.
    {
        // Load the first non-empty batch. An immediately-empty group (the DB
        // answered MT_DONE, or only zero-row batches, or a terminal event)
        // invokes `run()` zero times — a clean no-op, matching empty input.
        let mut first_input: Option<InputRowSet> = None;
        while let Some(table) = fetch().map_err(|e| RuntimeError::Udf(e.to_string()))? {
            let rs = InputRowSet::from_proto(&table, &meta.input_columns);
            if !rs.is_empty() {
                first_input = Some(rs);
                break;
            }
        }

        if let Some(mut input) = first_input {
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
            bridge.configure_group_input(meta.input_iter(), meta.output_iter(), Box::new(fetch));

            match meta.input_iter() {
                // SCALAR: framework-driven per-row loop. Invoke run() for the
                // current row, then advance the cursor (fetching the next batch
                // on drain) until the group boundary.
                IterType::ExactlyOnce => loop {
                    if let Err(e) = invoke_run(&mut bridge, udf) {
                        run_err = Some(e);
                        break;
                    }
                    match bridge.advance_row() {
                        Ok(true) => continue,
                        Ok(false) => break,
                        Err(e) => {
                            run_err = Some(RuntimeError::Udf(e.to_string()));
                            break;
                        }
                    }
                },
                // SET: UDF-driven per-group. Invoke run() once; ctx.next() spans
                // the group's batches and returns false at the boundary.
                IterType::Multiple => {
                    if let Err(e) = invoke_run(&mut bridge, udf) {
                        run_err = Some(e);
                    }
                }
            }
        }
    }

    if let Some(e) = run_err {
        return Err(e);
    }
    match exit.take() {
        Some(GroupExit::Session) => return Ok(Some(Ok(()))),
        Some(GroupExit::Closed(msg)) => return Ok(Some(close_error(msg))),
        None => {}
    }

    // Tail flush: always flush the group's residual output before its MT_DONE,
    // even if the byte threshold was never reached. Threshold crossings already
    // flushed mid-group inside `emit`, so this is only the trailing rows.
    if !emit_buf.is_empty() {
        emit_buf.record_flush_telemetry();
        let table = emit_buf.to_proto(&meta.output_columns);
        let mut proto = cell_ref.borrow_mut();
        let req = proto.emit_request(table);
        request(transport, &mut proto, req)?;
        emit_buf.clear();
    }

    Ok(None)
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

/// Invoke the UDF's `run` shim once over the current context view.
///
/// ABI contract: pass a pointer to a `&mut dyn UdfContext` (double
/// indirection). The run shim restores it via
/// `&mut *(ctx as *mut &mut dyn UdfContext)`.
fn invoke_run(bridge: &mut HostContextBridge, udf: &LoadedUdf) -> Result<(), RuntimeError> {
    let mut dyn_ref: &mut dyn UdfContext = &mut *bridge;
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
    Ok(())
}
