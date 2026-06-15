//! Test fixture cdylib that exercises the single-call vtable hooks.
//!
//! Unlike the `#[exasol_udf]` macro (which currently leaves all single-call
//! hooks `None`), this fixture wires `default_output_columns` and
//! `virtual_schema_adapter_call` directly so the runtime's single-call
//! dispatcher can be tested against a real `.so` boundary. The other two hooks
//! are left `None` to verify the runtime replies `MT_UNDEFINED_CALL` for them.

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

unsafe extern "C" fn run_shim(_ctx: *mut std::ffi::c_void) -> i32 {
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
    _ctx: *mut std::ffi::c_void,
    json_arg: *const c_char,
    result: *mut *mut c_char,
) -> i32 {
    unsafe {
        let arg = if json_arg.is_null() {
            String::new()
        } else {
            CStr::from_ptr(json_arg).to_string_lossy().into_owned()
        };
        write_result(&format!(r#"{{"echo":{arg}}}"#), result);
        0
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
    generate_sql_for_import_spec: None,
    generate_sql_for_export_spec: None,
    annotated_input_schema: std::ptr::null(),
    annotated_output_schema: std::ptr::null(),
};

#[unsafe(no_mangle)]
pub extern "C" fn __exa_udf_entry() -> *const exasol_udf_sdk::abi::ExaUdfVTable {
    &VTABLE as *const _
}
