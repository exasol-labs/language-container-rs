//! Verifies the `#[exasol_udf(import_spec(fn), export_spec(fn))]` annotations
//! wire the `generate_sql_for_import_spec` / `generate_sql_for_export_spec`
//! vtable slots to the named functions over the 3-argument
//! `(ctx, json_spec, result)` ABI, and that omitting them leaves both slots
//! `None` so the runtime still replies `MT_UNDEFINED_CALL`.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::test_support::DefaultsCtx;
use std::ffi::{CStr, CString, c_char, c_void};

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

type SpecSlot = unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_char) -> i32;

/// Drive a spec slot exactly as the host runtime does: a `&mut &mut dyn
/// UdfContext` erased to `*mut c_void`, and a `malloc`-backed result the caller
/// frees through the C allocator.
fn call_slot(hook: SpecSlot, json_spec: &str) -> (i32, String) {
    let mut ctx = DefaultsCtx;
    let mut dyn_ref: &mut dyn UdfContext = &mut ctx;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut c_void;

    let arg = CString::new(json_spec).unwrap();
    let mut out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { hook(ctx_ptr, arg.as_ptr(), &mut out) };
    assert!(!out.is_null(), "the shim must write a result string");
    let text = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(out as *mut c_void) };
    (rc, text)
}

fn import_sql(ctx: &mut dyn UdfContext, json_spec: &str) -> Result<String, UdfError> {
    Ok(format!(
        "SELECT * FROM {}.WORKER('{json_spec}')",
        ctx.script_schema()
    ))
}

fn export_sql(_ctx: &mut dyn UdfContext, json_spec: &str) -> Result<String, UdfError> {
    if json_spec.is_empty() {
        return Err(UdfError::Type("empty export specification".into()));
    }
    Ok(format!("SELECT WORKER('{json_spec}')"))
}

#[exasol_udf(import_spec(import_sql), export_spec(export_sql))]
fn spec_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf(name = "NAMED_SPEC", import_spec(import_sql))]
fn named_spec_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf]
fn plain_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[test]
fn spec_annotations_wire_both_vtable_slots() {
    let vt = unsafe { &*__exa_udf_entry_SPEC_RUN() };

    let import = vt
        .generate_sql_for_import_spec
        .expect("import_spec must wire the generate_sql_for_import_spec slot");
    let (rc, sql) = call_slot(import, r#"{"is_subselect":false}"#);
    assert_eq!(rc, 0);
    assert_eq!(sql, r#"SELECT * FROM .WORKER('{"is_subselect":false}')"#);

    let export = vt
        .generate_sql_for_export_spec
        .expect("export_spec must wire the generate_sql_for_export_spec slot");
    let (rc, sql) = call_slot(export, r#"{"has_truncate":true}"#);
    assert_eq!(rc, 0);
    assert_eq!(sql, r#"SELECT WORKER('{"has_truncate":true}')"#);

    let (rc, text) = call_slot(export, "");
    assert_eq!(rc, 1);
    assert!(text.contains("empty export specification"), "{text}");
}

#[test]
fn omitted_spec_annotations_leave_both_slots_none() {
    let vt = unsafe { &*__exa_udf_entry_PLAIN_RUN() };
    assert!(vt.generate_sql_for_import_spec.is_none());
    assert!(vt.generate_sql_for_export_spec.is_none());
    assert!(vt.virtual_schema_adapter_call.is_none());
}

#[test]
fn name_combines_with_spec_sections() {
    let vt = unsafe { &*__exa_udf_entry_NAMED_SPEC() };
    let import = vt
        .generate_sql_for_import_spec
        .expect("import_spec must wire its slot under a `name` override");
    let (rc, _) = call_slot(import, "{}");
    assert_eq!(rc, 0);
    assert!(vt.generate_sql_for_export_spec.is_none());
}
