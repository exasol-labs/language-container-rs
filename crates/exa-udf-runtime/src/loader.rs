use crate::error::RuntimeError;
use exa_zmq_protocol::IterType;
use exasol_udf_sdk::abi::{EXA_SDK_FINGERPRINT, EXA_UDF_ABI_VERSION, ExaUdfVTable, OutputShape};
use libloading::{Library, Symbol};

/// A loaded UDF shared object plus its validated vtable.
///
/// The `Library` is held alive for the whole session so the OS does not
/// `dlclose` the object (which would unmap the `run`/`destroy` code and the
/// `.rodata` the fingerprint pointer references) while the runtime still
/// dispatches into it.
pub struct LoadedUdf {
    _lib: Library,
    vtable: *const ExaUdfVTable,
}

// SAFETY: LoadedUdf is only used from a single thread; the runtime serializes
// all calls into the UDF. The raw vtable pointer is never shared concurrently.
unsafe impl Send for LoadedUdf {}

type EntryFn = unsafe extern "C" fn() -> *const ExaUdfVTable;

impl LoadedUdf {
    /// Open a `.so`, resolve `__exa_udf_entry_<script_name>`, and validate the
    /// vtable's ABI version and SDK fingerprint before returning a usable handle.
    ///
    /// `script_name` is the SQL object name the database sent in the handshake
    /// metadata — it is used verbatim to build the symbol name, so it must
    /// already be in the exact form the macro derived (i.e. UPPER_SNAKE_CASE).
    ///
    /// On any mismatch this returns an error WITHOUT calling `run` or `destroy`.
    pub fn open(path: &std::path::Path, script_name: &str) -> Result<Self, RuntimeError> {
        let lib = unsafe { Library::new(path) }?;

        let symbol_name = format!("__exa_udf_entry_{script_name}\0");
        let entry: Symbol<EntryFn> = unsafe { lib.get(symbol_name.as_bytes()) }.map_err(|_| {
            RuntimeError::Loader(format!(
                "no entry point found for script '{script_name}'; hint: rebuild with sdk >= 0.14.0"
            ))
        })?;

        let vtable_ptr = unsafe { entry() };
        if vtable_ptr.is_null() {
            return Err(RuntimeError::Loader(format!(
                "__exa_udf_entry_{script_name} returned null"
            )));
        }

        let vtable = unsafe { &*vtable_ptr };

        if vtable.abi_version != EXA_UDF_ABI_VERSION {
            return Err(RuntimeError::AbiMismatch {
                expected: EXA_UDF_ABI_VERSION,
                found: vtable.abi_version,
            });
        }

        let so_fp = unsafe {
            std::ffi::CStr::from_ptr(vtable.fingerprint)
                .to_str()
                .unwrap_or("")
        };
        // EXA_SDK_FINGERPRINT carries a trailing NUL for C interop; compare on
        // the NUL-free body so the &str comparison matches the CStr body.
        let host_fp = EXA_SDK_FINGERPRINT.trim_end_matches('\0');
        if so_fp != host_fp {
            return Err(RuntimeError::FingerprintMismatch {
                expected: host_fp.to_string(),
                found: so_fp.to_string(),
            });
        }

        Ok(LoadedUdf {
            _lib: lib,
            vtable: vtable_ptr,
        })
    }

    /// The compiled output shape the macro stamped into the vtable.
    ///
    /// Safe to read only on a vtable that passed [`LoadedUdf::open`]: the ABI
    /// version match there guarantees the `.so` shares the host's full vtable
    /// layout, so this field is present and holds a valid `OutputShape`.
    fn output_shape(&self) -> OutputShape {
        unsafe { &*self.vtable }.output_shape
    }

    /// Validate the compiled output shape against the DB's output iteration type,
    /// alongside the load-time ABI-version and fingerprint checks.
    ///
    /// `ExactlyOnce` output (RETURNS) must pair with a `.so` compiled as
    /// [`OutputShape::Returns`]; `Multiple` output (EMITS) with
    /// [`OutputShape::Emits`]. A mismatch (e.g. an emitting UDF registered
    /// RETURNS) is a clear error rather than a mid-stream misdispatch.
    pub fn validate_output_shape(&self, output_iter: IterType) -> Result<(), RuntimeError> {
        let compiled = self.output_shape();
        let registered = match output_iter {
            IterType::ExactlyOnce => OutputShape::Returns,
            IterType::Multiple => OutputShape::Emits,
        };
        if compiled != registered {
            return Err(RuntimeError::OutputShapeMismatch {
                compiled: shape_name(compiled),
                registered: shape_name(registered),
            });
        }
        Ok(())
    }

    /// Invoke the UDF's `run`.
    ///
    /// # Safety
    ///
    /// `ctx_ptr` must be a pointer to a live `&mut dyn UdfContext` (double
    /// indirection) per the ABI contract in `exasol_udf_sdk::abi`.
    ///
    /// `error_out` is a host-owned out-pointer to a `*mut c_char` initialised to
    /// null. On the user-error path the shim may write a `malloc`-allocated,
    /// NUL-terminated C string into it; the caller then owns and frees that
    /// string via `libc::free` (the C-allocator convention shared with the
    /// other single-call result strings). On the success and panic paths it is
    /// left untouched.
    pub unsafe fn run(
        &self,
        ctx_ptr: *mut std::ffi::c_void,
        error_out: *mut *mut std::ffi::c_char,
    ) -> i32 {
        let vtable = unsafe { &*self.vtable };
        unsafe { (vtable.run)(ctx_ptr, error_out) }
    }

    /// Invoke the UDF's `destroy`. Idempotency is the UDF's responsibility.
    ///
    /// # Safety
    ///
    /// Must be called at most once per `run` cycle and only after `run` has
    /// returned, per the ABI contract in `exasol_udf_sdk::abi`.
    pub unsafe fn destroy(&self) {
        let vtable = unsafe { &*self.vtable };
        unsafe { (vtable.destroy)() };
    }

    /// The annotated input schema JSON embedded in the vtable, or `None` when
    /// the UDF was not annotated with `input(...)`.
    pub(crate) fn annotated_input_schema(&self) -> Option<&str> {
        let vtable = unsafe { &*self.vtable };
        c_str_opt(vtable.annotated_input_schema)
    }

    /// The annotated output schema JSON embedded in the vtable, or `None` when
    /// the UDF was not annotated with `emits(...)`.
    pub(crate) fn annotated_output_schema(&self) -> Option<&str> {
        let vtable = unsafe { &*self.vtable };
        c_str_opt(vtable.annotated_output_schema)
    }

    /// Call the `default_output_columns` single-call hook.
    ///
    /// Returns `None` when the UDF did not register the hook, otherwise the
    /// hook's JSON result (or a [`RuntimeError`] on a non-zero return code).
    ///
    /// # Safety
    ///
    /// Only valid in single-call mode and only while the loaded `.so` is alive.
    pub unsafe fn call_default_output_columns(&self) -> Option<Result<String, RuntimeError>> {
        let vtable = unsafe { &*self.vtable };
        let hook = vtable.default_output_columns?;
        Some(unsafe { call_noarg_hook("default_output_columns", hook) })
    }

    /// Call the `virtual_schema_adapter_call` single-call hook with `json_arg`,
    /// threading the host context pointer so the adapter can call
    /// `ctx.connection(...)` / `ctx.connect_back(...)` during the call.
    ///
    /// # Safety
    ///
    /// In addition to [`LoadedUdf::call_default_output_columns`], `ctx` must be a
    /// pointer to a live `&mut dyn UdfContext` (double indirection) per the ABI
    /// contract in `exasol_udf_sdk::abi`, valid for the duration of the call.
    pub unsafe fn call_virtual_schema_adapter_call(
        &self,
        ctx: *mut std::ffi::c_void,
        json_arg: &str,
    ) -> Option<Result<String, RuntimeError>> {
        let vtable = unsafe { &*self.vtable };
        let hook = vtable.virtual_schema_adapter_call?;
        Some(unsafe { call_ctx_arg_hook("virtual_schema_adapter_call", ctx, json_arg, hook) })
    }

    /// Call the `generate_sql_for_import_spec` single-call hook.
    ///
    /// # Safety
    ///
    /// See [`LoadedUdf::call_default_output_columns`].
    pub unsafe fn call_generate_sql_for_import_spec(
        &self,
        json_spec: &str,
    ) -> Option<Result<String, RuntimeError>> {
        let vtable = unsafe { &*self.vtable };
        let hook = vtable.generate_sql_for_import_spec?;
        Some(unsafe { call_arg_hook("generate_sql_for_import_spec", json_spec, hook) })
    }

    /// Call the `generate_sql_for_export_spec` single-call hook.
    ///
    /// # Safety
    ///
    /// See [`LoadedUdf::call_default_output_columns`].
    pub unsafe fn call_generate_sql_for_export_spec(
        &self,
        json_spec: &str,
    ) -> Option<Result<String, RuntimeError>> {
        let vtable = unsafe { &*self.vtable };
        let hook = vtable.generate_sql_for_export_spec?;
        Some(unsafe { call_arg_hook("generate_sql_for_export_spec", json_spec, hook) })
    }
}

/// SQL-facing name for an output shape, used in the mismatch error message.
fn shape_name(shape: OutputShape) -> &'static str {
    match shape {
        OutputShape::Returns => "RETURNS",
        OutputShape::Emits => "EMITS",
    }
}

/// Convert a possibly-null `*const c_char` vtable field into a borrowed `&str`.
fn c_str_opt<'a>(ptr: *const std::ffi::c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }.to_str().ok()
}

/// Drive a no-argument single-call hook: invoke it, check the return code, and
/// take ownership of the heap-allocated result string it wrote.
unsafe fn call_noarg_hook(
    name: &str,
    hook: unsafe extern "C" fn(*mut *mut std::ffi::c_char) -> i32,
) -> Result<String, RuntimeError> {
    let mut out: *mut std::ffi::c_char = std::ptr::null_mut();
    let rc = unsafe { hook(&mut out) };
    if rc != 0 {
        let msg = unsafe { crate::single_call::take_c_string(out) };
        return Err(RuntimeError::Udf(if msg.is_empty() {
            format!("single-call hook {name} returned error code {rc}")
        } else {
            msg
        }));
    }
    Ok(unsafe { crate::single_call::take_c_string(out) })
}

/// Drive a single-argument single-call hook over a NUL-terminated JSON string.
unsafe fn call_arg_hook(
    name: &str,
    arg: &str,
    hook: unsafe extern "C" fn(*const std::ffi::c_char, *mut *mut std::ffi::c_char) -> i32,
) -> Result<String, RuntimeError> {
    let c_arg = std::ffi::CString::new(arg)
        .map_err(|_| RuntimeError::Udf(format!("{name}: argument contains interior NUL")))?;
    let mut out: *mut std::ffi::c_char = std::ptr::null_mut();
    let rc = unsafe { hook(c_arg.as_ptr(), &mut out) };
    if rc != 0 {
        let msg = unsafe { crate::single_call::take_c_string(out) };
        return Err(RuntimeError::Udf(if msg.is_empty() {
            format!("single-call hook {name} returned error code {rc}")
        } else {
            msg
        }));
    }
    Ok(unsafe { crate::single_call::take_c_string(out) })
}

/// Drive a context-plus-argument single-call hook: thread the host context
/// pointer (double-indirected `&mut dyn UdfContext`) and a NUL-terminated JSON
/// string into the hook, check the return code, and take ownership of the
/// heap-allocated result string it wrote.
unsafe fn call_ctx_arg_hook(
    name: &str,
    ctx: *mut std::ffi::c_void,
    arg: &str,
    hook: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        *const std::ffi::c_char,
        *mut *mut std::ffi::c_char,
    ) -> i32,
) -> Result<String, RuntimeError> {
    let c_arg = std::ffi::CString::new(arg)
        .map_err(|_| RuntimeError::Udf(format!("{name}: argument contains interior NUL")))?;
    let mut out: *mut std::ffi::c_char = std::ptr::null_mut();
    let rc = unsafe { hook(ctx, c_arg.as_ptr(), &mut out) };
    if rc != 0 {
        let msg = unsafe { crate::single_call::take_c_string(out) };
        return Err(RuntimeError::Udf(if msg.is_empty() {
            format!("single-call hook {name} returned error code {rc}")
        } else {
            msg
        }));
    }
    Ok(unsafe { crate::single_call::take_c_string(out) })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // Helpers shared by inline loader tests
    // ---------------------------------------------------------------------------

    fn compile_vtable_fixture(
        out_dir: &std::path::Path,
        name: &str,
        abi_version: u32,
    ) -> std::path::PathBuf {
        let src = format!(
            r#"
use std::ffi::c_char;
use std::os::raw::c_void;

#[repr(C)]
pub struct ExaUdfVTable {{
    pub abi_version: u32,
    pub fingerprint: *const c_char,
    pub run: unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32,
    pub destroy: unsafe extern "C" fn(),
}}
unsafe impl Sync for ExaUdfVTable {{}}

unsafe extern "C" fn run_stub(_ctx: *mut c_void, _out: *mut *mut c_char) -> i32 {{ 0 }}
unsafe extern "C" fn destroy_stub() {{}}

static FP: &str = "0.0.0:stub\0";
static VT: ExaUdfVTable = ExaUdfVTable {{
    abi_version: {abi_version},
    fingerprint: FP.as_ptr() as *const c_char,
    run: run_stub,
    destroy: destroy_stub,
}};

#[no_mangle]
pub extern "C" fn __exa_udf_entry_TESTABI() -> *const ExaUdfVTable {{
    &VT as *const ExaUdfVTable
}}
"#
        );
        let src_path = out_dir.join(format!("{name}.rs"));
        let so_path = out_dir.join(format!("lib{name}.so"));
        std::fs::write(&src_path, &src).expect("write fixture source");
        let status = std::process::Command::new("rustc")
            .arg("--crate-type=cdylib")
            .arg("--edition=2021")
            .arg("-o")
            .arg(&so_path)
            .arg(&src_path)
            .status()
            .expect("invoke rustc");
        assert!(status.success(), "rustc failed for {name}");
        so_path
    }

    /// Compile a fixture whose vtable mirrors the host's full `ExaUdfVTable`
    /// layout, including the trailing `output_shape` marker, so the loader may
    /// soundly read that field. `output_shape` is the raw `OutputShape`
    /// discriminant (0 = Returns, 1 = Emits). Uses the host fingerprint so the
    /// `.so` passes the ABI/fingerprint gate.
    fn compile_full_vtable_fixture(
        out_dir: &std::path::Path,
        name: &str,
        output_shape: u32,
    ) -> std::path::PathBuf {
        let abi = EXA_UDF_ABI_VERSION;
        let host_fp = EXA_SDK_FINGERPRINT.trim_end_matches('\0');
        let src = format!(
            r#"
use std::ffi::c_char;
use std::os::raw::c_void;

#[repr(u32)]
pub enum OutputShape {{ Returns = 0, Emits = 1 }}

#[repr(C)]
pub struct ExaUdfVTable {{
    pub abi_version: u32,
    pub fingerprint: *const c_char,
    pub run: unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32,
    pub destroy: unsafe extern "C" fn(),
    pub default_output_columns: Option<unsafe extern "C" fn(*mut *mut c_char) -> i32>,
    pub virtual_schema_adapter_call:
        Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_char) -> i32>,
    pub generate_sql_for_import_spec:
        Option<unsafe extern "C" fn(*const c_char, *mut *mut c_char) -> i32>,
    pub generate_sql_for_export_spec:
        Option<unsafe extern "C" fn(*const c_char, *mut *mut c_char) -> i32>,
    pub annotated_input_schema: *const c_char,
    pub annotated_output_schema: *const c_char,
    pub output_shape: OutputShape,
}}
unsafe impl Sync for ExaUdfVTable {{}}

unsafe extern "C" fn run_stub(_ctx: *mut c_void, _out: *mut *mut c_char) -> i32 {{ 0 }}
unsafe extern "C" fn destroy_stub() {{}}

static FP: &str = "{host_fp}\0";
static VT: ExaUdfVTable = ExaUdfVTable {{
    abi_version: {abi},
    fingerprint: FP.as_ptr() as *const c_char,
    run: run_stub,
    destroy: destroy_stub,
    default_output_columns: None,
    virtual_schema_adapter_call: None,
    generate_sql_for_import_spec: None,
    generate_sql_for_export_spec: None,
    annotated_input_schema: std::ptr::null(),
    annotated_output_schema: std::ptr::null(),
    output_shape: {output_shape_variant},
}};

#[no_mangle]
pub extern "C" fn __exa_udf_entry_SHAPE() -> *const ExaUdfVTable {{
    &VT as *const ExaUdfVTable
}}
"#,
            output_shape_variant = if output_shape == 0 {
                "OutputShape::Returns"
            } else {
                "OutputShape::Emits"
            },
        );
        let src_path = out_dir.join(format!("{name}.rs"));
        let so_path = out_dir.join(format!("lib{name}.so"));
        std::fs::write(&src_path, &src).expect("write fixture source");
        let status = std::process::Command::new("rustc")
            .arg("--crate-type=cdylib")
            .arg("--edition=2021")
            .arg("-o")
            .arg(&so_path)
            .arg(&src_path)
            .status()
            .expect("invoke rustc");
        assert!(status.success(), "rustc failed for {name}");
        so_path
    }

    /// An EMITS-compiled `.so` validates clean against `Multiple` output but is
    /// rejected against `ExactlyOnce` (RETURNS) with `OutputShapeMismatch`.
    #[test]
    fn output_shape_validated_against_output_iter() {
        let dir = make_tempdir();
        let so = compile_full_vtable_fixture(dir.path(), "shape_emits", 1);
        let udf = LoadedUdf::open(&so, "SHAPE").expect("full-vtable fixture must load");

        assert!(
            udf.validate_output_shape(IterType::Multiple).is_ok(),
            "EMITS marker must satisfy Multiple output"
        );
        match udf.validate_output_shape(IterType::ExactlyOnce) {
            Err(RuntimeError::OutputShapeMismatch {
                compiled,
                registered,
            }) => {
                assert_eq!(compiled, "EMITS");
                assert_eq!(registered, "RETURNS");
            }
            other => panic!("expected OutputShapeMismatch, got {other:?}"),
        }
    }

    fn make_tempdir() -> TempDir {
        let mut base = std::env::temp_dir();
        let unique = format!(
            "exa-loader-inline-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        base.push(unique);
        std::fs::create_dir_all(&base).expect("create tempdir");
        TempDir { path: base }
    }

    struct TempDir {
        path: std::path::PathBuf,
    }
    impl TempDir {
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    // ---------------------------------------------------------------------------
    // ABI-version tripwire: a .so built against v4 must be rejected, not misdispatched
    // ---------------------------------------------------------------------------

    /// A `.so` built against ABI version 4 (the pre-#31-fix vtable layout) must
    /// be rejected by the current loader with `AbiMismatch` — not loaded and
    /// silently misdispatched, which was the failure mode #31 was designed to
    /// prevent.
    #[test]
    fn current_abi_rejects_v4_so() {
        let dir = make_tempdir();
        let so = compile_vtable_fixture(dir.path(), "v4_fixture", 4);
        match LoadedUdf::open(&so, "TESTABI") {
            Err(RuntimeError::AbiMismatch { expected, found }) => {
                assert_eq!(
                    expected, EXA_UDF_ABI_VERSION,
                    "host must be the current ABI"
                );
                assert_eq!(found, 4, "fixture must present as ABI v4");
            }
            Err(other) => panic!("expected AbiMismatch, got {other:?}"),
            Ok(_) => panic!("loader must not accept a v4 .so against the current host"),
        }
    }

    /// Hook that writes a C-allocated error message into `*out` and returns 1.
    unsafe extern "C" fn hook_error_with_msg(out: *mut *mut std::ffi::c_char) -> i32 {
        unsafe {
            *out = libc::strdup(c"hook returned this error".as_ptr());
        }
        1
    }

    /// Hook that leaves `*out` null and returns 1 (no message available).
    unsafe extern "C" fn hook_error_null_out(out: *mut *mut std::ffi::c_char) -> i32 {
        let _ = out;
        1
    }

    /// Hook that writes a C-allocated result string into `*out` and returns 0.
    unsafe extern "C" fn hook_success(out: *mut *mut std::ffi::c_char) -> i32 {
        unsafe {
            *out = libc::strdup(c"the value".as_ptr());
        }
        0
    }

    #[test]
    fn error_text_surfaced_when_rc_nonzero() {
        let result = unsafe { call_noarg_hook("my_hook", hook_error_with_msg) };
        match result {
            Err(RuntimeError::Udf(msg)) => assert_eq!(msg, "hook returned this error"),
            other => panic!("expected Udf error, got {other:?}"),
        }
    }

    #[test]
    fn generic_message_when_error_text_empty() {
        let result = unsafe { call_noarg_hook("my_hook", hook_error_null_out) };
        match result {
            Err(RuntimeError::Udf(msg)) => {
                assert!(
                    msg.contains("returned error code"),
                    "expected generic fallback message, got: {msg}"
                );
            }
            other => panic!("expected Udf error, got {other:?}"),
        }
    }

    #[test]
    fn success_path_returns_written_string() {
        let result = unsafe { call_noarg_hook("my_hook", hook_success) };
        assert_eq!(result.unwrap(), "the value");
    }
}
