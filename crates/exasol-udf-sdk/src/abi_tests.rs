use super::*;

#[test]
fn abi_version_and_vtable_layout() {
    assert_eq!(EXA_UDF_ABI_VERSION, 7);
    assert!(std::mem::size_of::<ExaUdfVTable>() > 0);
    let _ = EXA_SDK_FINGERPRINT;
}

#[test]
fn vtable_layout_includes_vs_adapter() {
    // A vtable with all single-call hooks absent and no annotated schema
    // must still be constructible — the new fields are all nullable.
    unsafe extern "C" fn run_stub(
        _ctx: *mut std::ffi::c_void,
        _error_out: *mut *mut c_char,
    ) -> i32 {
        0
    }
    unsafe extern "C" fn destroy_stub() {}
    let vt = ExaUdfVTable {
        abi_version: EXA_UDF_ABI_VERSION,
        fingerprint: EXA_SDK_FINGERPRINT.as_ptr() as *const c_char,
        run: run_stub,
        destroy: destroy_stub,
        default_output_columns: None,
        virtual_schema_adapter_call: None,
        generate_sql_for_import_spec: None,
        generate_sql_for_export_spec: None,
        annotated_input_schema: std::ptr::null(),
        annotated_output_schema: std::ptr::null(),
        output_shape: OutputShape::Emits,
    };
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
fn vs_adapter_slot_receives_context_pointer() {
    // The virtual_schema_adapter_call slot must take a context pointer as its
    // FIRST argument so the VS adapter can call ctx.connection()/connect_back()
    // from single-call mode. This test pins the 3-arg ABI: (ctx, json, result).
    // Declared locally so the test does not pull the `libc` crate, which
    // would perturb dev-dependency resolution.
    unsafe extern "C" {
        fn free(ptr: *mut std::ffi::c_void);
    }
    unsafe extern "C" fn vsa(
        ctx: *mut std::ffi::c_void,
        _json: *const c_char,
        result: *mut *mut c_char,
    ) -> i32 {
        unsafe extern "C" {
            fn malloc(size: usize) -> *mut std::ffi::c_void;
        }
        // Echo whether a non-null context pointer was threaded through.
        let marker = if ctx.is_null() { b"0\0" } else { b"1\0" };
        let buf = unsafe { malloc(marker.len()) } as *mut c_char;
        unsafe {
            std::ptr::copy_nonoverlapping(marker.as_ptr() as *const c_char, buf, marker.len())
        };
        unsafe { *result = buf };
        0
    }
    unsafe extern "C" fn run_stub(
        _ctx: *mut std::ffi::c_void,
        _error_out: *mut *mut c_char,
    ) -> i32 {
        0
    }
    unsafe extern "C" fn destroy_stub() {}
    let vt = ExaUdfVTable {
        abi_version: EXA_UDF_ABI_VERSION,
        fingerprint: EXA_SDK_FINGERPRINT.as_ptr() as *const c_char,
        run: run_stub,
        destroy: destroy_stub,
        default_output_columns: None,
        virtual_schema_adapter_call: Some(vsa),
        generate_sql_for_import_spec: None,
        generate_sql_for_export_spec: None,
        annotated_input_schema: std::ptr::null(),
        annotated_output_schema: std::ptr::null(),
        output_shape: OutputShape::Returns,
    };
    let hook = vt.virtual_schema_adapter_call.unwrap();
    let mut ctx_byte = 0u8;
    let ctx_ptr = &mut ctx_byte as *mut u8 as *mut std::ffi::c_void;
    let arg = std::ffi::CString::new("{}").unwrap();
    let mut out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { hook(ctx_ptr, arg.as_ptr(), &mut out) };
    assert_eq!(rc, 0);
    let s = unsafe { std::ffi::CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(out as *mut std::ffi::c_void) };
    assert_eq!(s, "1", "the context pointer must be threaded to the slot");
}

#[test]
fn connect_back_types_compile_unconditionally() {
    // ConnectionObject and ExaConnection are always available — no feature gate.
    // Naming the types here fails to compile if the connect_back module ever
    // goes back behind a cargo feature (the #31 hazard).
    let _ = std::mem::size_of::<crate::connect_back::ConnectionObject>();
    fn _assert_trait_object(_: &dyn crate::connect_back::ExaConnection) {}
}
