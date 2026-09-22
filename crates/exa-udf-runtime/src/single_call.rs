use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use crate::wire::{close_error, request};
use exa_proto::SingleCallFunctionId;
use exa_zmq_protocol::{HostEvent, Protocol, UdfMeta, ZmqTransport};
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
    // receives a `SingleCallContext` (the virtual-schema adapter call and both
    // spec-generation hooks) surfaces the same live `exascript_info` values the
    // streaming `HostContextBridge` does, instead of the trait's neutral
    // defaults.
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
                fn_id,
                json_arg,
                import_spec,
                export_spec,
            } => {
                let call = SingleCallRequest {
                    fn_id,
                    json_arg,
                    import_spec,
                    export_spec,
                };
                let session = CallSession {
                    transport,
                    proto: &mut *proto,
                    handshake: handshake.clone(),
                };
                let outcome = invoke_hook(session, udf, call)?;
                let undefined = matches!(outcome, HookOutcome::Undefined);
                let reply = match outcome {
                    HookOutcome::Returned(result) => proto.return_request(result),
                    HookOutcome::Undefined => proto.undefined_call_request(hook_name(fn_id)),
                };
                // Send MT_RETURN/MT_UNDEFINED_CALL and consume the DB's ack,
                // which echoes the message just sent; a defensive MT_CLEANUP
                // ends the session early.
                match request(transport, proto, reply)? {
                    HostEvent::SingleCallAck if !undefined => {}
                    HostEvent::UndefinedCallAck if undefined => {}
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

/// The payload of one `MT_CALL`. Each single-call function id names its own
/// field on `exascript_single_call_rep`, so the dispatcher carries all of them
/// and reads the one its id selects.
struct SingleCallRequest {
    fn_id: SingleCallFunctionId,
    json_arg: Option<String>,
    import_spec: Option<exa_proto::ImportSpecificationRep>,
    export_spec: Option<exa_proto::ExportSpecificationRep>,
}

/// Everything one `MT_CALL` needs besides its own payload: the wire to reach the
/// DB on and the handshake snapshot a `SingleCallContext` is built from.
struct CallSession<'a> {
    transport: &'a ZmqTransport,
    proto: &'a mut Protocol,
    handshake: crate::rowset::HandshakeMeta,
}

fn invoke_hook(
    session: CallSession<'_>,
    udf: &LoadedUdf,
    call: SingleCallRequest,
) -> Result<HookOutcome, RuntimeError> {
    match call.fn_id {
        SingleCallFunctionId::ScFnDefaultOutputColumns => {
            hook_outcome(unsafe { udf.call_default_output_columns() })
        }
        SingleCallFunctionId::ScFnVirtualSchemaAdapterCall => {
            let arg = call.json_arg.unwrap_or_default();
            invoke_ctx_hook(session, &arg, |ctx, arg| unsafe {
                udf.call_virtual_schema_adapter_call(ctx, arg)
            })
        }
        SingleCallFunctionId::ScFnGenerateSqlForImportSpec => {
            if !udf.implements_import_spec_hook() {
                return Ok(HookOutcome::Undefined);
            }
            let spec = call.import_spec.ok_or_else(|| {
                missing_specification("generate_sql_for_import_spec", "import_specification")
            })?;
            let json = crate::spec_json::serialize_import(&spec);
            invoke_ctx_hook(session, &json, |ctx, arg| unsafe {
                udf.call_generate_sql_for_import_spec(ctx, arg)
            })
        }
        SingleCallFunctionId::ScFnGenerateSqlForExportSpec => {
            if !udf.implements_export_spec_hook() {
                return Ok(HookOutcome::Undefined);
            }
            let spec = call.export_spec.ok_or_else(|| {
                missing_specification("generate_sql_for_export_spec", "export_specification")
            })?;
            let json = crate::spec_json::serialize_export(&spec);
            invoke_ctx_hook(session, &json, |ctx, arg| unsafe {
                udf.call_generate_sql_for_export_spec(ctx, arg)
            })
        }
        SingleCallFunctionId::ScFnNil => Ok(HookOutcome::Undefined),
    }
}

/// A spec-generation call whose own specification field is unpopulated is a
/// malformed exchange: the hook would otherwise build SQL from an empty
/// payload, so the session closes naming the field the database left out.
fn missing_specification(hook: &str, field: &str) -> RuntimeError {
    RuntimeError::Udf(format!(
        "single-call hook {hook}: the MT_CALL carried no {field} message"
    ))
}

fn hook_outcome(result: Option<Result<String, RuntimeError>>) -> Result<HookOutcome, RuntimeError> {
    match result {
        Some(Ok(s)) => Ok(HookOutcome::Returned(s)),
        Some(Err(e)) => Err(e),
        None => Ok(HookOutcome::Undefined),
    }
}

fn invoke_ctx_hook<F>(
    session: CallSession<'_>,
    arg: &str,
    call: F,
) -> Result<HookOutcome, RuntimeError>
where
    F: FnOnce(*mut std::ffi::c_void, &str) -> Option<Result<String, RuntimeError>>,
{
    let CallSession {
        transport,
        proto,
        handshake,
    } = session;

    // `transport`/`proto` feed the on-demand MT_IMPORT closure only when
    // connect-back is enabled; without it the context's `connection()` /
    // `connect_back()` inherit the trait's Unimplemented defaults.
    #[cfg(not(feature = "connect-back"))]
    let _ = (transport, proto);

    #[cfg(feature = "connect-back")]
    let proto_cell = std::cell::RefCell::new(proto);

    #[cfg(feature = "connect-back")]
    let conn_requester: crate::rowset::ConnRequester =
        crate::wire::conn_requester(transport, &proto_cell);

    let mut bridge = crate::rowset::SingleCallContext::new(
        handshake,
        #[cfg(feature = "connect-back")]
        conn_requester,
    );
    // ABI contract: pass a pointer to a `&mut dyn UdfContext` (double
    // indirection), exactly as the run loop does.
    let mut dyn_ref: &mut dyn UdfContext = &mut bridge;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;
    match call(ctx_ptr, arg) {
        Some(Ok(s)) => Ok(HookOutcome::Returned(s)),
        Some(Err(e)) => Err(match bridge.take_last_error() {
            Some(detail) => RuntimeError::Udf(format!("{e}: {detail}")),
            None => e,
        }),
        None => Ok(HookOutcome::Undefined),
    }
}

/// The name reported in `MT_UNDEFINED_CALL`: the SDK hook an author would
/// implement, so the database's "function not implemented" diagnostic names
/// something actionable. `SC_FN_NIL` is a sentinel naming no hook, so it keeps
/// the protobuf variant name.
fn hook_name(fn_id: SingleCallFunctionId) -> &'static str {
    match fn_id {
        SingleCallFunctionId::ScFnDefaultOutputColumns => "default_output_columns",
        SingleCallFunctionId::ScFnVirtualSchemaAdapterCall => "virtual_schema_adapter_call",
        SingleCallFunctionId::ScFnGenerateSqlForImportSpec => "generate_sql_for_import_spec",
        SingleCallFunctionId::ScFnGenerateSqlForExportSpec => "generate_sql_for_export_spec",
        SingleCallFunctionId::ScFnNil => fn_id.as_str_name(),
    }
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
