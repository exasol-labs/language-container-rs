use super::*;
use std::ffi::c_void;

// `free`/`malloc` are declared locally so the test does not pull the `libc`
// crate, which would perturb dev-dependency resolution.
unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

unsafe extern "C" fn run_stub(_ctx: *mut c_void, _error_out: *mut *mut c_char) -> i32 {
    0
}

fn vtable_without_hooks() -> ExaUdfVTable {
    ExaUdfVTable {
        abi_version: EXA_UDF_ABI_VERSION,
        fingerprint: EXA_SDK_FINGERPRINT.as_ptr() as *const c_char,
        run: run_stub,
        cleanup: None,
        default_output_columns: None,
        virtual_schema_adapter_call: None,
        generate_sql_for_import_spec: None,
        generate_sql_for_export_spec: None,
        annotated_input_schema: std::ptr::null(),
        annotated_output_schema: std::ptr::null(),
        output_shape: OutputShape::Emits,
    }
}

unsafe fn write_ctx_presence(ctx: *mut c_void, out: *mut *mut c_char) {
    let marker = if ctx.is_null() { b"0\0" } else { b"1\0" };
    let buf = unsafe { malloc(marker.len()) } as *mut c_char;
    unsafe { std::ptr::copy_nonoverlapping(marker.as_ptr() as *const c_char, buf, marker.len()) };
    unsafe { *out = buf };
}

fn take_c_string(ptr: *mut c_char) -> String {
    let text = unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(ptr as *mut c_void) };
    text
}

#[test]
fn vtable_layout_includes_vs_adapter() {
    // A vtable with all single-call hooks absent and no annotated schema
    // must still be constructible — the new fields are all nullable.
    let vt = vtable_without_hooks();
    assert!(vt.virtual_schema_adapter_call.is_none());
    assert!(vt.annotated_input_schema.is_null());
    assert!(vt.annotated_output_schema.is_null());
}

// The fingerprint is a compile-time const, so clippy can prove these checks
// statically. That is exactly the point: the assertions verify build.rs ran
// and baked a non-empty "SDK_VERSION:RUSTC_HASH" value into the binary.
#[test]
#[allow(clippy::const_is_empty)]
fn fingerprint_baked_nonempty() {
    assert!(!EXA_SDK_FINGERPRINT.is_empty());
    assert!(EXA_SDK_FINGERPRINT.contains(':'));
}

#[test]
fn spec_slots_take_context() {
    // Every context-taking single-call slot must take a context pointer as its
    // FIRST argument so the hook can call ctx.connection()/connect_back() and
    // read handshake metadata from single-call mode. This pins the 3-arg ABI
    // `(ctx, json, result)` on all three slots at once, so a slot that regressed
    // to the 2-arg shape fails to compile here.
    unsafe extern "C" fn echo_ctx_presence(
        ctx: *mut c_void,
        _json: *const c_char,
        result: *mut *mut c_char,
    ) -> i32 {
        unsafe { write_ctx_presence(ctx, result) };
        0
    }
    let vt = ExaUdfVTable {
        virtual_schema_adapter_call: Some(echo_ctx_presence),
        generate_sql_for_import_spec: Some(echo_ctx_presence),
        generate_sql_for_export_spec: Some(echo_ctx_presence),
        ..vtable_without_hooks()
    };

    let slots = [
        (
            "virtual_schema_adapter_call",
            vt.virtual_schema_adapter_call,
        ),
        (
            "generate_sql_for_import_spec",
            vt.generate_sql_for_import_spec,
        ),
        (
            "generate_sql_for_export_spec",
            vt.generate_sql_for_export_spec,
        ),
    ];
    for (name, slot) in slots {
        let hook = slot.unwrap();
        let mut ctx_byte = 0u8;
        let ctx_ptr = &mut ctx_byte as *mut u8 as *mut c_void;
        let arg = std::ffi::CString::new("{}").unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = unsafe { hook(ctx_ptr, arg.as_ptr(), &mut out) };
        assert_eq!(rc, 0);
        assert_eq!(
            take_c_string(out),
            "1",
            "{name} must receive the context pointer"
        );
    }
}

#[test]
fn cleanup_slot_takes_context_and_abi_version_is_eleven() {
    assert_eq!(EXA_UDF_ABI_VERSION, 11);
    let word = std::mem::size_of::<usize>();
    assert_eq!(
        std::mem::offset_of!(ExaUdfVTable, cleanup),
        std::mem::offset_of!(ExaUdfVTable, run) + word
    );
    assert_eq!(
        std::mem::offset_of!(ExaUdfVTable, default_output_columns),
        std::mem::offset_of!(ExaUdfVTable, cleanup) + word
    );
    unsafe extern "C" fn fail_with_ctx_presence(
        ctx: *mut c_void,
        error_out: *mut *mut c_char,
    ) -> i32 {
        unsafe { write_ctx_presence(ctx, error_out) };
        1
    }
    let vt = ExaUdfVTable {
        cleanup: Some(fail_with_ctx_presence),
        ..vtable_without_hooks()
    };

    let cleanup = vt.cleanup.expect("the cleanup slot must hold the hook");
    let mut ctx_byte = 0u8;
    let ctx_ptr = &mut ctx_byte as *mut u8 as *mut c_void;
    let mut error_out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { cleanup(ctx_ptr, &mut error_out) };

    assert_eq!(rc, 1);
    assert_eq!(
        take_c_string(error_out),
        "1",
        "cleanup must receive the context pointer"
    );
}

#[test]
fn connect_back_types_compile_unconditionally() {
    // ConnectionObject and ExaConnection are always available — no feature gate.
    // Naming the types here fails to compile if the connect_back module ever
    // goes back behind a cargo feature (the #31 hazard).
    let _ = std::mem::size_of::<crate::connect_back::ConnectionObject>();
    fn _assert_trait_object(_: &dyn crate::connect_back::ExaConnection) {}
}
