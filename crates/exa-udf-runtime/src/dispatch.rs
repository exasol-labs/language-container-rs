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
/// then pulls each input batch with `MT_NEXT`, flushes a batch's output with a
/// single `MT_EMIT` (batching), and finally sends `MT_DONE`. The DB's reply to
/// each request classifies what to do next.
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
                let emitted = run_batch(udf, &table, meta)?;
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
            | HostEvent::Ping(_) => {}
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
fn run_batch(
    udf: &LoadedUdf,
    table: &exa_proto::ExascriptTableData,
    meta: &UdfMeta,
) -> Result<EmitBuffer, RuntimeError> {
    let mut input = InputRowSet::from_proto(table, &meta.input_columns);
    let mut emit_buf = EmitBuffer::new();
    {
        let mut bridge = HostContextBridge::new(&mut input, &mut emit_buf, &meta.input_columns);
        // ABI contract: pass a pointer to a `&mut dyn UdfContext` (double
        // indirection). The run shim restores it via
        // `&mut *(ctx as *mut &mut dyn UdfContext)`.
        let mut dyn_ref: &mut dyn UdfContext = &mut bridge;
        let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;
        let rc = unsafe { udf.run(ctx_ptr) };
        if rc != 0 {
            return Err(RuntimeError::Udf(format!(
                "UDF run returned error code {rc}"
            )));
        }
    }
    Ok(emit_buf)
}
