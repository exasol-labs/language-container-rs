use crate::error::RuntimeError;
use exasol_udf_sdk::abi::{EXA_SDK_FINGERPRINT, EXA_UDF_ABI_VERSION, ExaUdfVTable};
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
    /// Open a `.so`, resolve `__exa_udf_entry`, and validate the vtable's ABI
    /// version and SDK fingerprint before returning a usable handle.
    ///
    /// On any mismatch this returns an error WITHOUT calling `run` or `destroy`.
    pub fn open(path: &std::path::Path) -> Result<Self, RuntimeError> {
        let lib = unsafe { Library::new(path) }?;
        let entry: Symbol<EntryFn> = unsafe { lib.get(b"__exa_udf_entry\0") }
            .map_err(|e| RuntimeError::Loader(format!("symbol not found: {e}")))?;

        let vtable_ptr = unsafe { entry() };
        if vtable_ptr.is_null() {
            return Err(RuntimeError::Loader("__exa_udf_entry returned null".into()));
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

    /// Invoke the UDF's `run`.
    ///
    /// # Safety
    ///
    /// `ctx_ptr` must be a pointer to a live `&mut dyn UdfContext` (double
    /// indirection) per the ABI contract in `exasol_udf_sdk::abi`.
    pub unsafe fn run(&self, ctx_ptr: *mut std::ffi::c_void) -> i32 {
        let vtable = unsafe { &*self.vtable };
        unsafe { (vtable.run)(ctx_ptr) }
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
    pub fn annotated_input_schema(&self) -> Option<&str> {
        let vtable = unsafe { &*self.vtable };
        c_str_opt(vtable.annotated_input_schema)
    }

    /// The annotated output schema JSON embedded in the vtable, or `None` when
    /// the UDF was not annotated with `emits(...)`.
    pub fn annotated_output_schema(&self) -> Option<&str> {
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
        if !out.is_null() {
            unsafe { libc::free(out as *mut libc::c_void) };
        }
        return Err(RuntimeError::Udf(format!(
            "single-call hook {name} returned error code {rc}"
        )));
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
        if !out.is_null() {
            unsafe { libc::free(out as *mut libc::c_void) };
        }
        return Err(RuntimeError::Udf(format!(
            "single-call hook {name} returned error code {rc}"
        )));
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
        if !out.is_null() {
            unsafe { libc::free(out as *mut libc::c_void) };
        }
        return Err(RuntimeError::Udf(format!(
            "single-call hook {name} returned error code {rc}"
        )));
    }
    Ok(unsafe { crate::single_call::take_c_string(out) })
}
