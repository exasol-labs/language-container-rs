use std::ffi::c_char;

/// ABI version — bump only when the vtable layout changes.
pub const EXA_UDF_ABI_VERSION: u32 = 7;

/// Compiled output shape of a UDF, stamped into the vtable so the host can
/// validate it against the DB's `output_iter_type` at load/run time.
///
/// The discriminants mirror the protocol `IterType`: an `ExactlyOnce` output
/// iteration (RETURNS, one value per invocation) is `0`; a `Multiple` output
/// iteration (EMITS, any number of rows) is `1`. Declared `#[repr(u32)]` so the
/// marker is a stable, C-ABI-safe scalar that crosses the `.so` boundary
/// unambiguously.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputShape {
    /// RETURNS: the UDF returns one value per invocation via `set_return`.
    Returns = 0,
    /// EMITS: the UDF emits any number of output rows via `emit`.
    Emits = 1,
}

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
    /// Compiled output shape derived from the UDF function's return type
    /// (`Result<(), _>` ⇒ EMITS, `Result<Option<T>, _>` ⇒ RETURNS). The host
    /// validates this against the DB's `output_iter_type` at load/run time.
    pub output_shape: OutputShape,
}

// Safety: we only send the vtable pointer across thread boundaries controlled by the runtime,
// never concurrently — the host runtime serializes all UDF calls.
unsafe impl Send for ExaUdfVTable {}
unsafe impl Sync for ExaUdfVTable {}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;
