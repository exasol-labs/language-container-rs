use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use exa_proto::SingleCallFunctionId;
use exa_zmq_protocol::{HostAction, HostEvent, Protocol, UdfMeta, ZmqTransport};
use exasol_udf_sdk::context::UdfContext;
use std::ffi::{CStr, c_char};

/// Drive a single-call session.
///
/// In single-call mode the DB does not stream input/output batches. Instead it
/// answers each `MT_RUN` with one `MT_CALL` naming a single-call function, and
/// expects exactly one `MT_RETURN` (the function's JSON result) or
/// `MT_UNDEFINED_CALL` (the function is not implemented in this container) per
/// call. The session ends when the DB answers `MT_RUN` (or a call's reply) with
/// `MT_CLEANUP`, after which the client sends `MT_FINISHED`.
///
/// The wire stays in strict REQ/REP lockstep: every request the client sends is
/// answered by exactly one DB response. A function call therefore costs two
/// exchanges — one to receive the `MT_CALL`, one to send the result and receive
/// the next directive.
pub fn run_single_call(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    meta: &UdfMeta,
) -> Result<(), RuntimeError> {
    // Snapshot the handshake metadata once so every single-call hook that
    // receives a `SingleCallContext` (the virtual-schema adapter call) surfaces
    // the same live `exascript_info` values the streaming `HostContextBridge`
    // does, instead of the trait's neutral defaults.
    let handshake = crate::rowset::HandshakeMeta::from(meta);
    // Mirror the canonical C++ single-call loop:
    //   loop { MT_RUN -> MT_CALL; dispatch; MT_RETURN/-UNDEFINED; MT_DONE }
    //   then MT_FINISHED.
    // The DB acknowledges the container's MT_RETURN with MT_RETURN (not
    // MT_CLEANUP); the session only ends when the DB answers a later MT_RUN or
    // MT_DONE with MT_CLEANUP.
    loop {
        match request(transport, proto, proto.run_request())? {
            HostEvent::SingleCall {
                fn_id, json_arg, ..
            } => {
                let reply = match invoke_hook(
                    transport,
                    proto,
                    udf,
                    fn_id,
                    json_arg.as_deref(),
                    handshake.clone(),
                )? {
                    HookOutcome::Returned(result) => proto.return_request(result),
                    HookOutcome::Undefined => proto.undefined_call_request(fn_name(fn_id)),
                };
                // Send MT_RETURN/MT_UNDEFINED_CALL and consume the DB's ack.
                // The ack is MT_RETURN (`SingleCallAck`); a defensive MT_CLEANUP
                // ends the session early.
                match request(transport, proto, reply)? {
                    HostEvent::SingleCallAck => {}
                    HostEvent::Cleanup => break,
                    HostEvent::Close(msg) => return close_error(msg),
                    other => return unexpected(other),
                }
            }
            // The DB ends the session by answering MT_RUN with MT_CLEANUP.
            HostEvent::Cleanup => break,
            HostEvent::Close(msg) => return close_error(msg),
            other => return unexpected(other),
        }

        // Close the run with MT_DONE; the DB answers MT_DONE to continue or
        // MT_CLEANUP to end the session.
        match request(transport, proto, proto.done_request())? {
            HostEvent::Done => {}
            HostEvent::Cleanup => break,
            HostEvent::Close(msg) => return close_error(msg),
            other => return unexpected(other),
        }
    }

    // Client-initiated teardown: MT_FINISHED, then the DB echoes it.
    request(transport, proto, proto.finished_reply())?;
    Ok(())
}

/// No other message is valid at this point in single-call mode. Retrying the
/// wire here would risk a livelock, so surface it as a hard error.
fn unexpected(event: HostEvent) -> Result<(), RuntimeError> {
    Err(RuntimeError::Udf(format!(
        "unexpected message in single-call mode: {event:?}"
    )))
}

/// The result of routing one `MT_CALL` to a vtable hook.
enum HookOutcome {
    Returned(String),
    Undefined,
}

/// Route a single-call function id to its vtable hook, returning the hook's
/// string result or `Undefined` when the UDF did not register the hook.
///
/// `transport` and `proto` are used only by `virtual_schema_adapter_call`, which
/// is handed a [`SingleCallContext`] so the adapter can call
/// `ctx.connection(name)` (an on-demand MT_IMPORT exchange) and
/// `ctx.connect_back(...)` during the call. The dispatch loop is blocked waiting
/// here, so the ZMQ socket is idle and the MT_IMPORT exchange is safe to perform.
fn invoke_hook(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    fn_id: SingleCallFunctionId,
    json_arg: Option<&str>,
    handshake: crate::rowset::HandshakeMeta,
) -> Result<HookOutcome, RuntimeError> {
    let arg = json_arg.unwrap_or("");
    let result = match fn_id {
        SingleCallFunctionId::ScFnDefaultOutputColumns => unsafe {
            udf.call_default_output_columns()
        },
        SingleCallFunctionId::ScFnVirtualSchemaAdapterCall => {
            return invoke_vs_adapter_call(transport, proto, udf, arg, handshake);
        }
        SingleCallFunctionId::ScFnGenerateSqlForImportSpec => unsafe {
            udf.call_generate_sql_for_import_spec(arg)
        },
        SingleCallFunctionId::ScFnGenerateSqlForExportSpec => unsafe {
            udf.call_generate_sql_for_export_spec(arg)
        },
        SingleCallFunctionId::ScFnNil => return Ok(HookOutcome::Undefined),
    };
    match result {
        Some(Ok(s)) => Ok(HookOutcome::Returned(s)),
        Some(Err(e)) => Err(e),
        None => Ok(HookOutcome::Undefined),
    }
}

/// Invoke the `virtual_schema_adapter_call` hook with a live [`SingleCallContext`]
/// threaded through the ABI's double-indirection, so the adapter can resolve
/// CONNECTION credentials and open self-connections mid-call.
fn invoke_vs_adapter_call(
    transport: &ZmqTransport,
    proto: &mut Protocol,
    udf: &LoadedUdf,
    arg: &str,
    handshake: crate::rowset::HandshakeMeta,
) -> Result<HookOutcome, RuntimeError> {
    // `transport`/`proto` feed the on-demand MT_IMPORT closure only when
    // connect-back is enabled; without it the context's `connection()` /
    // `connect_back()` inherit the trait's Unimplemented defaults.
    #[cfg(not(feature = "connect-back"))]
    let _ = (transport, proto);

    #[cfg(feature = "connect-back")]
    let proto_cell = std::cell::RefCell::new(proto);

    #[cfg(feature = "connect-back")]
    let conn_requester: crate::rowset::ConnRequester = Box::new(move |conn_name: &str| {
        let mut proto = proto_cell.borrow_mut();
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
            HostEvent::ConnInfo(ci) => Ok(ci),
            _ => Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "MT_IMPORT reply was not ConnInfo".into(),
            )),
        }
    });

    let mut bridge = crate::rowset::SingleCallContext::new(
        handshake,
        #[cfg(feature = "connect-back")]
        conn_requester,
    );
    // ABI contract: pass a pointer to a `&mut dyn UdfContext` (double
    // indirection), exactly as the run loop does.
    let mut dyn_ref: &mut dyn UdfContext = &mut bridge;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;
    let result = unsafe { udf.call_virtual_schema_adapter_call(ctx_ptr, arg) };
    match result {
        Some(Ok(s)) => Ok(HookOutcome::Returned(s)),
        Some(Err(e)) => Err(match bridge.take_last_error() {
            Some(detail) => RuntimeError::Udf(format!("{e}: {detail}")),
            None => e,
        }),
        None => Ok(HookOutcome::Undefined),
    }
}

/// The wire name reported in `MT_UNDEFINED_CALL` for a single-call function.
fn fn_name(fn_id: SingleCallFunctionId) -> &'static str {
    fn_id.as_str_name()
}

fn close_error(msg: Option<String>) -> Result<(), RuntimeError> {
    Err(RuntimeError::Udf(
        msg.unwrap_or_else(|| "connection closed by database".into()),
    ))
}

/// Send one request and return the classified response event, replying to a
/// ping transparently (REQ stays in lockstep: a ping reply is itself a
/// request/reply).
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

/// Consume a heap-allocated C string produced by a vtable single-call hook.
///
/// ABI contract: the hook allocates the result with `libc::malloc` (e.g. via a
/// `CString` copied into a `malloc`ed buffer) and transfers ownership to the
/// runtime through `*result`. The runtime copies it into an owned `String` and
/// frees the original with `libc::free`, so allocation and deallocation always
/// cross the boundary through the C allocator and never mix Rust's global
/// allocator with the UDF's.
pub(crate) unsafe fn take_c_string(ptr: *mut c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let owned = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { libc::free(ptr as *mut libc::c_void) };
    owned
}
