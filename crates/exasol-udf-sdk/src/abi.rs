use std::ffi::c_char;

/// ABI version — bump only when the vtable layout changes
pub const EXA_UDF_ABI_VERSION: u32 = 6;

/// The fingerprint string baked in at SDK build time; injected by build.rs.
/// Format: "SDK_VERSION:RUSTC_HASH\0". The build script supplies the
/// "SDK_VERSION:RUSTC_HASH" body (env vars cannot carry NUL bytes); the
/// trailing NUL terminator is appended here so the pointer is a valid C string.
pub const EXA_SDK_FINGERPRINT: &str = concat!(env!("EXA_SDK_FINGERPRINT"), "\0");

/// The vtable crossing the C ABI boundary between the host runtime and the UDF .so
/// All function pointers use extern "C" calling convention
/// repr(C) ensures stable layout across compilation units
#[repr(C)]
pub struct ExaUdfVTable {
    pub abi_version: u32,
    /// Null-terminated fingerprint string (points into .rodata of the .so)
    pub fingerprint: *const std::ffi::c_char,
    /// The UDF's run function. The `ctx` argument is a thin `*mut c_void`, but
    /// the UDF needs a fat `&mut dyn UdfContext`. The ABI contract is therefore
    /// double-indirection: the host runtime constructs
    /// `let mut r: &mut dyn UdfContext = &mut bridge;` and passes
    /// `&mut r as *mut _ as *mut c_void`. The run shim restores it via
    /// `&mut *(ctx as *mut &mut dyn UdfContext)`. The UDF must not store the
    /// pointer beyond this call. Returns 0 = ok, 1 = user error, 2 = panic.
    ///
    /// `error_out` is a pointer to a caller-provided `*mut c_char` initialised
    /// to null. On the error-return path (`1`) the shim MAY write a
    /// `malloc`-allocated, NUL-terminated C string into `*error_out`; the host
    /// then takes ownership of that string and frees it with `libc::free` — the
    /// same C-allocator convention as the other single-call result strings, so
    /// the `.so`'s and host's separately-linked Rust allocators are never
    /// mixed. On the `0` and `2` return paths the shim leaves `*error_out`
    /// untouched (null).
    pub run: unsafe extern "C" fn(ctx: *mut std::ffi::c_void, error_out: *mut *mut c_char) -> i32,
    /// Destroy the UDF instance (called after run). No-op for v1 stateless UDFs.
    pub destroy: unsafe extern "C" fn(),
    /// Single-call hook: emit the default output columns as a JSON string.
    /// `None` when the UDF does not implement it. On success writes a
    /// heap-allocated, caller-freed C string to `*result` and returns 0.
    pub default_output_columns: Option<unsafe extern "C" fn(result: *mut *mut c_char) -> i32>,
    /// Single-call hook: virtual-schema adapter call. `ctx` is the same
    /// double-indirected `&mut dyn UdfContext` pointer the host passes to `run`,
    /// so the adapter can call `ctx.connection(...)` / `ctx.connect_back(...)`
    /// during the call. `json_arg` is the request payload; the response C string
    /// is written to `*result`. `None` when not implemented.
    pub virtual_schema_adapter_call: Option<
        unsafe extern "C" fn(
            ctx: *mut std::ffi::c_void,
            json_arg: *const c_char,
            result: *mut *mut c_char,
        ) -> i32,
    >,
    /// Single-call hook: generate the SQL for an IMPORT spec. `None` when not
    /// implemented.
    pub generate_sql_for_import_spec:
        Option<unsafe extern "C" fn(json_spec: *const c_char, result: *mut *mut c_char) -> i32>,
    /// Single-call hook: generate the SQL for an EXPORT spec. `None` when not
    /// implemented.
    pub generate_sql_for_export_spec:
        Option<unsafe extern "C" fn(json_spec: *const c_char, result: *mut *mut c_char) -> i32>,
    /// Null-terminated JSON describing the annotated input schema, or NULL when
    /// the UDF was not annotated with `input(...)`.
    pub annotated_input_schema: *const c_char,
    /// Null-terminated JSON describing the annotated output schema, or NULL when
    /// the UDF was not annotated with `emits(...)`.
    pub annotated_output_schema: *const c_char,
}

// Safety: we only send the vtable pointer across thread boundaries controlled by the runtime,
// never concurrently — the host runtime serializes all UDF calls.
unsafe impl Send for ExaUdfVTable {}
unsafe impl Sync for ExaUdfVTable {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_and_vtable_layout() {
        assert_eq!(EXA_UDF_ABI_VERSION, 6);
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
}
