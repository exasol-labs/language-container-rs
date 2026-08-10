use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use crate::rowset::{BatchFetcher, EmitBuffer, EmitFlusher, HostContextBridge, InputRowSet};
use crate::wire::{close_error, request};
use exa_zmq_protocol::{ColumnMeta, HostEvent, IterType, Protocol, UdfMeta, ZmqTransport};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
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
/// `transport` and `proto` are shared via a single `RefCell` among the batch
/// fetcher, the emit flusher, the tail flush and the credential fetcher. The
/// borrows never overlap: UDF execution is single-threaded and each closure
/// holds the cell for one send/recv exchange only.
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

    let proto_cell = RefCell::new(proto);
    let cell_ref = &proto_cell;

    let mut fetch = batch_fetcher(transport, cell_ref, &exit);
    let mut run_err: Option<RuntimeError> = None;

    // The bridge's borrow of `emit_buf` ends with this block, freeing it for
    // the tail flush.
    if let Some(mut input) = first_nonempty_input(&mut fetch, &meta.input_columns)? {
        let mut bridge = HostContextBridge::new(
            &mut input,
            &mut emit_buf,
            &meta.input_columns,
            &meta.output_columns,
            emit_flusher(transport, cell_ref),
            crate::rowset::HandshakeMeta::from(meta),
            #[cfg(feature = "connect-back")]
            crate::wire::conn_requester(transport, cell_ref),
        );
        bridge.configure_group_input(meta.input_iter(), meta.output_iter(), fetch);
        run_err = drive_group_rows(&mut bridge, udf, meta.input_iter());
    }

    if let Some(e) = run_err {
        return Err(e);
    }
    match exit.take() {
        Some(GroupExit::Session) => return Ok(Some(Ok(()))),
        Some(GroupExit::Closed(msg)) => return Ok(Some(close_error(msg))),
        None => {}
    }

    tail_flush(&mut emit_buf, meta, transport, cell_ref)?;
    Ok(None)
}

/// Send one pre-built proto table as `MT_EMIT`. A zero-row table is a no-op, so
/// no zero-row `MT_EMIT` ever reaches the wire.
fn emit_flusher<'a>(
    transport: &'a ZmqTransport,
    proto_cell: &'a RefCell<&'a mut Protocol>,
) -> EmitFlusher<'a> {
    Box::new(
        move |table: exa_proto::ExascriptTableData| -> Result<(), UdfError> {
            if table.rows == 0 {
                return Ok(());
            }
            let mut proto = proto_cell.borrow_mut();
            let req = proto.emit_request(table);
            request(transport, &mut proto, req)
                .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
            Ok(())
        },
    )
}

/// Pull the next `MT_NEXT` batch: `Ok(Some)` a batch, `Ok(None)` the group
/// boundary (`MT_DONE`).
///
/// A mid-input `MT_CLEANUP` / `MT_CLOSE` records its reason in `exit` and
/// reports the group as ended — the fetcher runs inside `run()` via
/// `ctx.next()` and cannot unwind the session itself. `run_group` reads `exit`
/// once the UDF returns.
fn batch_fetcher<'a>(
    transport: &'a ZmqTransport,
    proto_cell: &'a RefCell<&'a mut Protocol>,
    exit: &'a Cell<Option<GroupExit>>,
) -> BatchFetcher<'a> {
    Box::new(
        move || -> Result<Option<exa_proto::ExascriptTableData>, UdfError> {
            loop {
                let mut proto = proto_cell.borrow_mut();
                let req = proto.next_request();
                let event = request(transport, &mut proto, req)
                    .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
                match event {
                    HostEvent::NextData(table) => return Ok(Some(table)),
                    HostEvent::Done => return Ok(None),
                    HostEvent::TryAgain | HostEvent::Reset => continue,
                    HostEvent::Cleanup => {
                        exit.set(Some(GroupExit::Session));
                        return Ok(None);
                    }
                    HostEvent::Close(msg) => {
                        exit.set(Some(GroupExit::Closed(msg)));
                        return Ok(None);
                    }
                    _ => continue,
                }
            }
        },
    )
}

/// Advance to the first row-bearing input batch, skipping zero-row ones.
///
/// `None` means the group delivered no rows, so `run()` is invoked zero times.
fn first_nonempty_input(
    fetch: &mut BatchFetcher,
    input_cols: &[ColumnMeta],
) -> Result<Option<InputRowSet>, RuntimeError> {
    while let Some(table) = fetch().map_err(|e| RuntimeError::Udf(e.to_string()))? {
        let rows = InputRowSet::from_proto(&table, input_cols);
        if !rows.is_empty() {
            return Ok(Some(rows));
        }
    }
    Ok(None)
}

/// Drive the UDF over one group, returning the error that ended it early.
///
/// `ExactlyOnce` (SCALAR) is framework-driven: `run()` per row, advancing the
/// cursor until the group boundary. `Multiple` (SET) is UDF-driven: `run()`
/// once, with `ctx.next()` spanning the group's batches.
fn drive_group_rows(
    bridge: &mut HostContextBridge,
    udf: &LoadedUdf,
    input_iter: IterType,
) -> Option<RuntimeError> {
    match input_iter {
        IterType::ExactlyOnce => loop {
            if let Err(e) = invoke_run(bridge, udf) {
                return Some(e);
            }
            match bridge.advance_row() {
                Ok(true) => continue,
                Ok(false) => return None,
                Err(e) => return Some(RuntimeError::Udf(e.to_string())),
            }
        },
        IterType::Multiple => invoke_run(bridge, udf).err(),
    }
}

/// Flush the group's residual output as one `MT_EMIT` before its `MT_DONE`,
/// even if the byte threshold was never reached.
fn tail_flush(
    emit_buf: &mut EmitBuffer,
    meta: &UdfMeta,
    transport: &ZmqTransport,
    proto_cell: &RefCell<&mut Protocol>,
) -> Result<(), RuntimeError> {
    if emit_buf.is_empty() {
        return Ok(());
    }
    emit_buf.record_flush_telemetry();
    let table = emit_buf.to_proto(&meta.output_columns);
    let mut proto = proto_cell.borrow_mut();
    let req = proto.emit_request(table);
    request(transport, &mut proto, req)?;
    emit_buf.clear();
    Ok(())
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
