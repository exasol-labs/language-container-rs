use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use crate::rowset::{BatchFetcher, EmitBuffer, EmitFlusher, HostContextBridge, InputRowSet};
use crate::wire::{close_error, request};
use exa_zmq_protocol::{ColumnMeta, HostEvent, IterType, Protocol, UdfMeta, ZmqTransport};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use std::cell::{Cell, RefCell};

pub fn run_udf(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    meta: &UdfMeta,
) -> Result<(), RuntimeError> {
    let mut emit_buf = EmitBuffer::new();
    let exit: Cell<Option<GroupExit>> = Cell::new(None);
    let proto_cell = RefCell::new(proto);
    let wire = SessionWire {
        transport,
        proto_cell: &proto_cell,
        exit: &exit,
    };

    loop {
        let event = {
            let mut p = wire.proto_cell.borrow_mut();
            let req = p.run_request();
            request(transport, &mut p, req)?
        };
        match event {
            HostEvent::Run => {}
            HostEvent::Cleanup => break,
            HostEvent::Close(msg) => return close_error(msg),
            _ => {}
        }

        emit_buf.group_reset();
        wire.exit.set(None);

        if let Some(early) = run_group(&wire, &mut emit_buf, udf, meta)? {
            return early;
        }

        let event = {
            let mut p = wire.proto_cell.borrow_mut();
            let req = p.done_request();
            request(transport, &mut p, req)?
        };
        match event {
            HostEvent::Done => {}
            HostEvent::Cleanup => break,
            HostEvent::Close(msg) => return close_error(msg),
            _ => {}
        }
    }

    {
        let mut p = wire.proto_cell.borrow_mut();
        let req = p.finished_reply();
        request(transport, &mut p, req)?;
    }
    Ok(())
}

/// How a group's input driving ended, when not the normal group boundary.
enum GroupExit {
    /// The DB answered `MT_CLEANUP` mid-input: end the whole session cleanly.
    Session,
    /// The DB answered `MT_CLOSE` mid-input: surface the exception message.
    Closed(Option<String>),
}

/// The session's wire plumbing, bundled since every group-scoped helper needs
/// all three together.
struct SessionWire<'s> {
    transport: &'s ZmqTransport,
    proto_cell: &'s RefCell<&'s mut Protocol>,
    exit: &'s Cell<Option<GroupExit>>,
}

fn run_group<'s>(
    wire: &SessionWire<'s>,
    emit_buf: &mut EmitBuffer,
    udf: &LoadedUdf,
    meta: &'s UdfMeta,
) -> Result<Option<Result<(), RuntimeError>>, RuntimeError> {
    let mut fetch = batch_fetcher(wire);
    let mut run_err: Option<RuntimeError> = None;

    if let Some(mut input) = first_nonempty_input(&mut fetch, &meta.input_columns)? {
        emit_buf.reserve_rows(input.rows_in_group());
        let mut bridge = HostContextBridge::new(
            &mut input,
            emit_buf,
            &meta.input_columns,
            &meta.output_columns,
            emit_flusher(wire),
            crate::rowset::HandshakeMeta::from(meta),
            #[cfg(feature = "connect-back")]
            crate::wire::conn_requester(wire.transport, wire.proto_cell),
        );
        bridge.configure_group_input(meta.input_iter(), meta.output_iter(), fetch);
        run_err = drive_group_rows(&mut bridge, udf, meta.input_iter());
    }

    if let Some(e) = run_err {
        return Err(e);
    }
    match wire.exit.take() {
        Some(GroupExit::Session) => return Ok(Some(Ok(()))),
        Some(GroupExit::Closed(msg)) => return Ok(Some(close_error(msg))),
        None => {}
    }

    tail_flush(emit_buf, meta, wire)?;
    Ok(None)
}

/// Send one pre-built proto table as `MT_EMIT`. A zero-row table is a no-op, so
/// no zero-row `MT_EMIT` ever reaches the wire.
fn emit_flusher<'a>(wire: &SessionWire<'a>) -> EmitFlusher<'a> {
    let transport = wire.transport;
    let proto_cell = wire.proto_cell;
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
fn batch_fetcher<'a>(wire: &SessionWire<'a>) -> BatchFetcher<'a> {
    let transport = wire.transport;
    let proto_cell = wire.proto_cell;
    let exit = wire.exit;
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
        let rows = InputRowSet::from_proto(table, input_cols);
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
    wire: &SessionWire,
) -> Result<(), RuntimeError> {
    if emit_buf.is_empty() {
        return Ok(());
    }
    emit_buf.record_flush_telemetry();
    let table = emit_buf.to_proto(&meta.output_columns);
    let mut proto = wire.proto_cell.borrow_mut();
    let req = proto.emit_request(table);
    request(wire.transport, &mut proto, req)?;
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
