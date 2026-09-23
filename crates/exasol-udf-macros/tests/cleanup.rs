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

fn first_cleanup(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Err(UdfError::User("first cleanup ran".into()))
}

fn second_cleanup(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Err(UdfError::User("second cleanup ran".into()))
}

#[exasol_udf(cleanup(cleanup_by_script_name))]
fn outcome_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf]
fn plain_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

// Fails to compile if the macro emits a cleanup shim without `cleanup(...)`.
#[allow(dead_code, non_snake_case)]
fn __exa_cleanup_shim_PLAIN_RUN() {}

#[exasol_udf(cleanup(cleanup_by_script_name))]
fn tidy_up_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf(cleanup(cleanup_by_script_name), name = "NAMED_CLEANUP")]
fn named_cleanup_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf(cleanup(first_cleanup))]
fn first_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf(cleanup(second_cleanup))]
fn second_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn slot_address(slot: Option<CleanupSlot>) -> Option<usize> {
    slot.map(|f| f as usize)
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

#[test]
fn cleanup_shim_carries_the_entry_suffix() {
    let vt = unsafe { &*__exa_udf_entry_TIDY_UP_RUN() };
    assert_eq!(
        slot_address(vt.cleanup),
        slot_address(Some(__exa_cleanup_shim_TIDY_UP_RUN))
    );
}

#[test]
fn name_combines_with_cleanup_section() {
    let vt = unsafe { &*__exa_udf_entry_NAMED_CLEANUP() };
    assert_eq!(
        slot_address(vt.cleanup),
        slot_address(Some(__exa_cleanup_shim_NAMED_CLEANUP))
    );
    assert_eq!(
        call_cleanup(vt.cleanup.unwrap(), "FAIL"),
        (1, Some("cleanup of FAIL failed".to_string()))
    );
}

#[test]
fn distinct_entries_get_independent_cleanup_shims() {
    let first = unsafe { &*__exa_udf_entry_FIRST_RUN() };
    let second = unsafe { &*__exa_udf_entry_SECOND_RUN() };

    assert_eq!(
        call_cleanup(first.cleanup.unwrap(), ""),
        (1, Some("first cleanup ran".to_string()))
    );
    assert_eq!(
        call_cleanup(second.cleanup.unwrap(), ""),
        (1, Some("second cleanup ran".to_string()))
    );
}
