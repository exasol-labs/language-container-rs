//! Verifies the `#[exasol_udf]` run shim surfaces a UDF error through the
//! `error_out` pointer using the C allocator convention: on the user-error
//! return path (`1`) it writes a `malloc`-allocated, NUL-terminated C string
//! that the host frees with `free` — never `CString::into_raw`/`from_raw`,
//! which would mix the `.so`'s and host's separate Rust allocators (UB for
//! statically-linked musl `.so`s).

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::ffi::{CStr, c_char};

unsafe extern "C" {
    fn free(ptr: *mut std::ffi::c_void);
}

#[exasol_udf]
fn failing_run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Err(UdfError::Type("boom".into()))
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
fn run_shim_writes_malloc_backed_error_string_on_user_error() {
    let vt = unsafe { &*__exa_udf_entry() };

    let mut ctx = NoopCtx;
    let mut dyn_ref: &mut dyn UdfContext = &mut ctx;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;

    let mut error_out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { (vt.run)(ctx_ptr, &mut error_out) };
    assert_eq!(rc, 1, "user error must return code 1");
    assert!(
        !error_out.is_null(),
        "run shim must write an error string on the user-error path"
    );

    let msg = unsafe { CStr::from_ptr(error_out) }
        .to_string_lossy()
        .into_owned();
    // The shim allocates with `malloc`; free through the C allocator (declared
    // above), mirroring the host's `libc::free`. Freeing a Rust-`into_raw`
    // pointer with `free` would be heap corruption — this asserts the C
    // allocator convention holds.
    unsafe { free(error_out as *mut std::ffi::c_void) };

    assert!(
        msg.contains("boom"),
        "error string must carry the UDF error message, got {msg:?}"
    );
}
