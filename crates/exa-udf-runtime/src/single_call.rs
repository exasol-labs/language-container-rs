use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use exa_proto::SingleCallFunctionId;
use exa_zmq_protocol::{HostAction, HostEvent, Protocol, UdfMeta, ZmqTransport};
use std::ffi::{c_char, CStr};

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
    _meta: &UdfMeta,
) -> Result<(), RuntimeError> {
    // Open the phase, mirroring the scalar/set loop.
    let mut event = request(transport, proto, proto.run_request())?;

    loop {
        match event {
            HostEvent::SingleCall {
                fn_id, json_arg, ..
            } => {
                let reply = match invoke_hook(udf, fn_id, json_arg.as_deref())? {
                    HookOutcome::Returned(result) => proto.return_request(result),
                    HookOutcome::Undefined => proto.undefined_call_request(fn_name(fn_id)),
                };
                event = request(transport, proto, reply)?;
            }
            // The DB ends the session by answering MT_RUN (or a call's reply)
            // with MT_CLEANUP.
            HostEvent::Cleanup => break,
            HostEvent::Close(msg) => return close_error(msg),
            // No other message is valid in single-call mode. Retrying the wire
            // here would risk a livelock, so surface it as a hard error.
            other => {
                return Err(RuntimeError::Udf(format!(
                    "unexpected message in single-call mode: {other:?}"
                )));
            }
        }
    }

    // Client-initiated teardown: MT_FINISHED, then the DB echoes it.
    request(transport, proto, proto.finished_reply())?;
    Ok(())
}

/// The result of routing one `MT_CALL` to a vtable hook.
enum HookOutcome {
    Returned(String),
    Undefined,
}

/// Route a single-call function id to its vtable hook, returning the hook's
/// string result or `Undefined` when the UDF did not register the hook.
fn invoke_hook(
    udf: &LoadedUdf,
    fn_id: SingleCallFunctionId,
    json_arg: Option<&str>,
) -> Result<HookOutcome, RuntimeError> {
    let arg = json_arg.unwrap_or("");
    let result = match fn_id {
        SingleCallFunctionId::ScFnDefaultOutputColumns => unsafe {
            udf.call_default_output_columns()
        },
        SingleCallFunctionId::ScFnVirtualSchemaAdapterCall => unsafe {
            udf.call_virtual_schema_adapter_call(arg)
        },
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
