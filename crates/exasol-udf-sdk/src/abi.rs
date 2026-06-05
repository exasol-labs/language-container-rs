/// ABI version — bump only when the vtable layout changes
pub const EXA_UDF_ABI_VERSION: u32 = 1;

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
    pub run: unsafe extern "C" fn(ctx: *mut std::ffi::c_void) -> i32,
    /// Destroy the UDF instance (called after run). No-op for v1 stateless UDFs.
    pub destroy: unsafe extern "C" fn(),
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
        assert_eq!(EXA_UDF_ABI_VERSION, 1);
        assert!(std::mem::size_of::<ExaUdfVTable>() > 0);
        let _ = EXA_SDK_FINGERPRINT;
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

    #[cfg(feature = "connect-back")]
    #[test]
    fn connect_back_feature_is_noop() {
        // The connect-back feature is declared for v1 but carries no behavior.
        // This test exists only to verify the crate compiles with it enabled.
        assert_eq!(EXA_UDF_ABI_VERSION, 1);
    }
}
