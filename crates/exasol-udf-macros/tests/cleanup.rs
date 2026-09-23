use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::test_support::TestContext;
use std::ffi::{CStr, c_char, c_void};

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

type CleanupSlot = unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32;

/// Call a cleanup slot the way the host does.
fn call_cleanup(slot: CleanupSlot, script_name: &str) -> (i32, Option<String>) {
    let mut ctx = TestContext::set(vec![]).with_script_name(script_name);
    let mut dyn_ref: &mut dyn UdfContext = &mut ctx;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut c_void;

    let mut error_out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { slot(ctx_ptr, &mut error_out) };
    if error_out.is_null() {
        return (rc, None);
    }
    let text = unsafe { CStr::from_ptr(error_out) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(error_out as *mut c_void) };
    (rc, Some(text))
}

fn cleanup_by_script_name(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    match ctx.script_name().as_str() {
        "FAIL" => Err(UdfError::User("cleanup of FAIL failed".into())),
        "PANIC" => panic!("cleanup of PANIC panicked"),
        _ => Ok(()),
    }
}

#[exasol_udf(cleanup(cleanup_by_script_name))]
fn outcome_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf]
fn plain_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[test]
fn cleanup_annotation_wires_slot_and_maps_outcomes() {
    let vt = unsafe { &*__exa_udf_entry_OUTCOME_RUN() };
    let cleanup = vt.cleanup.expect("cleanup(...) must wire the cleanup slot");

    assert_eq!(call_cleanup(cleanup, "OK"), (0, None));
    assert_eq!(
        call_cleanup(cleanup, "FAIL"),
        (1, Some("cleanup of FAIL failed".to_string()))
    );
    assert_eq!(
        call_cleanup(cleanup, "PANIC"),
        (2, None),
        "a panic must leave error_out untouched"
    );

    let mut ctx = TestContext::set(vec![]).with_script_name("FAIL");
    let mut dyn_ref: &mut dyn UdfContext = &mut ctx;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut c_void;
    let rc = unsafe { cleanup(ctx_ptr, std::ptr::null_mut()) };
    assert_eq!(rc, 1, "a null error_out still reports the user error");
}

#[test]
fn omitted_cleanup_leaves_slot_none() {
    let vt = unsafe { &*__exa_udf_entry_PLAIN_RUN() };
    assert!(vt.cleanup.is_none());
}
