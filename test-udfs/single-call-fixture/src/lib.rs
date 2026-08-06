//! Test fixture cdylib that exercises the single-call vtable hooks.
//!
//! Unlike the `#[exasol_udf]` macro (which currently leaves all single-call
//! hooks `None`), this fixture wires `default_output_columns`,
//! `virtual_schema_adapter_call`, and `generate_sql_for_import_spec` directly
//! so the runtime's single-call dispatcher can be tested against a real `.so`
//! boundary. `generate_sql_for_export_spec` alone is left `None` to verify the
//! runtime replies `MT_UNDEFINED_CALL` for it.

use exasol_udf_sdk::context::UdfContext;
use std::ffi::{CStr, CString, c_char};

/// Hand a Rust string to the runtime through a `libc::malloc`-backed buffer.
///
/// The runtime takes ownership and frees it with `libc::free`, so allocation
/// crosses the boundary entirely through the C allocator (never mixing Rust's
/// global allocator with the runtime's).
unsafe fn write_result(value: &str, out: *mut *mut c_char) {
    unsafe {
        let c = CString::new(value).expect("no interior NUL in fixture output");
        let bytes = c.as_bytes_with_nul();
        let buf = libc::malloc(bytes.len()) as *mut c_char;
        std::ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, bytes.len());
        *out = buf;
    }
}

unsafe extern "C" fn run_shim(_ctx: *mut std::ffi::c_void, _error_out: *mut *mut c_char) -> i32 {
    0
}

unsafe extern "C" fn destroy_shim() {}

unsafe extern "C" fn default_output_columns(result: *mut *mut c_char) -> i32 {
    unsafe {
        write_result(r#"[{"name":"c0","type":"Int64"}]"#, result);
        0
    }
}

unsafe extern "C" fn virtual_schema_adapter_call(
    ctx: *mut std::ffi::c_void,
    json_arg: *const c_char,
    result: *mut *mut c_char,
) -> i32 {
    unsafe {
        // Restore the host context via the ABI's double indirection. The runtime
        // builds `ctx` as `&mut (&mut dyn UdfContext) as *mut _ as *mut c_void`
        // (see `invoke_vs_adapter_call` in exa-udf-runtime/src/single_call.rs and
        // the `call_ctx_arg_hook` contract in loader.rs), so we cast back to
        // `*mut &mut dyn UdfContext` and dereference twice.
        let ctx: &mut dyn UdfContext = &mut **(ctx as *mut &mut dyn UdfContext);
        let arg = if json_arg.is_null() {
            ""
        } else {
            CStr::from_ptr(json_arg).to_str().unwrap_or("")
        };

        // Covers the runtime's success arm, which the deliberate-failure
        // default below never reaches.
        if arg == "SUCCEED" {
            write_result("VS_ADAPTER_OK", result);
            return 0;
        }

        // ctx.connection() records its own error on the context, independent of
        // this hook's failure below, so the runtime combines both into one
        // close message.
        if arg == "CONNECTION_PROBE" {
            let _ = ctx.connection("PROBE_CONN");
            write_result("VS_ADAPTER_ERROR", result);
            return 1;
        }

        // Surface the live handshake metadata through the deliberate-error
        // channel: the runtime reads this string off the `result` out-pointer
        // when the hook returns a non-zero code.
        write_result(
            &format!(
                "HANDSHAKE_META node_count={} node_id={} session_id={} script_name={}",
                ctx.node_count(),
                ctx.node_id(),
                ctx.session_id(),
                ctx.script_name(),
            ),
            result,
        );
        1
    }
}

/// Deliberately fails, echoing the received spec: exercises the runtime's
/// `ScFnGenerateSqlForImportSpec` arm and its hook-error mapping.
unsafe extern "C" fn generate_sql_for_import_spec(
    json_spec: *const c_char,
    result: *mut *mut c_char,
) -> i32 {
    unsafe {
        let spec = if json_spec.is_null() {
            String::new()
        } else {
            CStr::from_ptr(json_spec).to_string_lossy().into_owned()
        };
        write_result(&format!("IMPORT_SPEC_HOOK_ERROR arg={spec}"), result);
        1
    }
}

#[used]
static VTABLE: exasol_udf_sdk::abi::ExaUdfVTable = exasol_udf_sdk::abi::ExaUdfVTable {
    abi_version: exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
    fingerprint: exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT.as_ptr() as *const c_char,
    run: run_shim,
    destroy: destroy_shim,
    default_output_columns: Some(default_output_columns),
    virtual_schema_adapter_call: Some(virtual_schema_adapter_call),
    generate_sql_for_import_spec: Some(generate_sql_for_import_spec),
    generate_sql_for_export_spec: None,
    annotated_input_schema: std::ptr::null(),
    annotated_output_schema: std::ptr::null(),
    output_shape: exasol_udf_sdk::abi::OutputShape::Emits,
};

#[unsafe(no_mangle)]
pub extern "C" fn __exa_udf_entry_SINGLE_CALL_UDF() -> *const exasol_udf_sdk::abi::ExaUdfVTable {
    &VTABLE as *const _
}
