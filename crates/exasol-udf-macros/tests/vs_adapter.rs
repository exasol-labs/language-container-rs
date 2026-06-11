//! Verifies the `#[exasol_udf(vs_adapter(fn))]` annotation wires the
//! `virtual_schema_adapter_call` vtable slot to the named function and that the
//! slot receives a `UdfContext` pointer as its first argument (the new 3-arg
//! ABI), so a VS adapter can call `ctx.connection(...)` / `ctx.connect_back(...)`
//! from inside single-call mode.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::ffi::{c_char, CStr, CString};

extern "C" {
    fn free(ptr: *mut std::ffi::c_void);
}

fn adapter_echo(_ctx: &mut dyn UdfContext, json_arg: &str) -> Result<String, UdfError> {
    Ok(format!(r#"{{"echo":{json_arg}}}"#))
}

#[exasol_udf(vs_adapter(adapter_echo))]
fn vs_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

struct NoopCtx;

impl UdfContext for NoopCtx {
    fn num_columns(&self) -> usize {
        0
    }
    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Type("none".into()))
    }
    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }
}

#[test]
fn vs_adapter_annotation_wires_slot_and_echoes_through_context_abi() {
    let vt = unsafe { &*__exa_udf_entry() };
    let hook = vt
        .virtual_schema_adapter_call
        .expect("vs_adapter annotation must wire the virtual_schema_adapter_call slot");

    // Build the double-indirected context pointer exactly as the host runtime
    // does for `run`: a `&mut &mut dyn UdfContext` erased to `*mut c_void`.
    let mut ctx = NoopCtx;
    let mut dyn_ref: &mut dyn UdfContext = &mut ctx;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;

    let arg = CString::new("{}").unwrap();
    let mut out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { hook(ctx_ptr, arg.as_ptr(), &mut out) };
    assert_eq!(rc, 0, "hook must return 0 on success");
    assert!(!out.is_null(), "hook must write a result string");

    let result = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    // The shim allocates the result with `malloc`; free it through the C
    // allocator (declared above) so this test does not depend on the `libc`
    // crate, which would perturb the dev-dependency resolution.
    unsafe { free(out as *mut std::ffi::c_void) };
    assert_eq!(result, r#"{"echo":{}}"#);
}
