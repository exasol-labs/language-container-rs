use crate::error::RuntimeError;
use exa_zmq_protocol::IterType;
use exasol_udf_sdk::abi::{EXA_SDK_FINGERPRINT, EXA_UDF_ABI_VERSION, ExaUdfVTable, OutputShape};
use exasol_udf_sdk::context::UdfContext;
use libloading::{Library, Symbol};

/// A loaded UDF shared object plus its validated vtable.
///
/// The `Library` is held alive for the whole session so the OS does not
/// `dlclose` the object (which would unmap the `run`/`cleanup` code and the
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
    /// On any mismatch this returns an error WITHOUT calling `run` or `cleanup`.
    pub fn open(path: &std::path::Path, script_name: &str) -> Result<Self, RuntimeError> {
        let lib = unsafe { Library::new(path) }?;

        let symbol_name = format!("__exa_udf_entry_{script_name}\0");
        let entry: Symbol<EntryFn> = unsafe { lib.get(symbol_name.as_bytes()) }.map_err(|_| {
            // Lead with the most common cause — a name mismatch — because the
            // symbol is derived from the SQL script name, not the file: a plain
            // `fn run` exports `__exa_udf_entry_RUN`, so a script named e.g.
            // SCALAR_DOUBLE finds nothing. The SDK-version hint is the fallback
            // case (an .so predating entry-point naming), not the first guess.
            RuntimeError::Loader(format!(
                "no entry point found for script '{script_name}': the UDF .so exports no \
                 '__exa_udf_entry_{script_name}' symbol. The script name must match the UDF's \
                 exported entry — a plain `fn run` exports `__exa_udf_entry_RUN`, and \
                 `#[exasol_udf(name = \"...\")]` overrides it — so rename the script or the \
                 function so they agree. If the .so exports no `__exa_udf_entry_*` symbol at \
                 all, it predates entry-point naming: rebuild with sdk >= 0.14.0."
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

    /// Invoke the UDF's `run` over `ctx`.
    ///
    /// A non-zero return code becomes `RuntimeError::Udf` carrying
    /// `UDF run returned error code <rc>`, followed by `: <text>` when the shim
    /// wrote an error message. The success path allocates nothing, because a
    /// SCALAR group calls this once per input row.
    pub fn run(&self, ctx: &mut dyn UdfContext) -> Result<(), RuntimeError> {
        let vtable = unsafe { &*self.vtable };
        unsafe { call_lifecycle_slot("run", vtable.run, ctx) }
    }

    /// Invoke the UDF's cleanup hook over `ctx`, or return `None` when the UDF
    /// registered none. A failure reads `UDF cleanup returned error code <rc>`
    /// in the format [`LoadedUdf::run`] documents.
    pub fn cleanup(&self, ctx: &mut dyn UdfContext) -> Option<Result<(), RuntimeError>> {
        let vtable = unsafe { &*self.vtable };
        let slot = vtable.cleanup?;
        Some(unsafe { call_lifecycle_slot("cleanup", slot, ctx) })
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

    /// Whether the `.so` registered `generate_sql_for_import_spec`.
    ///
    /// The dispatcher asks before it decodes the specification payload, so an
    /// `IMPORT` call against a UDF that implements no import hook still answers
    /// `MT_UNDEFINED_CALL` and lets the database raise its own "function not
    /// implemented" diagnostic.
    pub(crate) fn implements_import_spec_hook(&self) -> bool {
        unsafe { &*self.vtable }
            .generate_sql_for_import_spec
            .is_some()
    }

    /// Whether the `.so` registered `generate_sql_for_export_spec`. See
    /// [`LoadedUdf::implements_import_spec_hook`].
    pub(crate) fn implements_export_spec_hook(&self) -> bool {
        unsafe { &*self.vtable }
            .generate_sql_for_export_spec
            .is_some()
    }

    /// Call the `generate_sql_for_import_spec` single-call hook with the
    /// serialized specification, threading the host context pointer so the hook
    /// can qualify the worker script with `ctx.script_schema()` and resolve
    /// CONNECTION credentials while it builds the SQL.
    ///
    /// # Safety
    ///
    /// See [`LoadedUdf::call_virtual_schema_adapter_call`].
    pub unsafe fn call_generate_sql_for_import_spec(
        &self,
        ctx: *mut std::ffi::c_void,
        json_spec: &str,
    ) -> Option<Result<String, RuntimeError>> {
        let vtable = unsafe { &*self.vtable };
        let hook = vtable.generate_sql_for_import_spec?;
        Some(unsafe { call_ctx_arg_hook("generate_sql_for_import_spec", ctx, json_spec, hook) })
    }

    /// Call the `generate_sql_for_export_spec` single-call hook. Same context
    /// contract as [`LoadedUdf::call_generate_sql_for_import_spec`].
    ///
    /// # Safety
    ///
    /// See [`LoadedUdf::call_virtual_schema_adapter_call`].
    pub unsafe fn call_generate_sql_for_export_spec(
        &self,
        ctx: *mut std::ffi::c_void,
        json_spec: &str,
    ) -> Option<Result<String, RuntimeError>> {
        let vtable = unsafe { &*self.vtable };
        let hook = vtable.generate_sql_for_export_spec?;
        Some(unsafe { call_ctx_arg_hook("generate_sql_for_export_spec", ctx, json_spec, hook) })
    }
}

type LifecycleSlot = unsafe extern "C" fn(*mut std::ffi::c_void, *mut *mut std::ffi::c_char) -> i32;

/// Call a `(ctx, error_out) -> i32` slot through the double-indirected context
/// pointer the ABI prescribes, and take ownership of any error text it wrote.
unsafe fn call_lifecycle_slot(
    slot_name: &'static str,
    slot: LifecycleSlot,
    ctx: &mut dyn UdfContext,
) -> Result<(), RuntimeError> {
    let mut ctx_ref: &mut dyn UdfContext = ctx;
    let ctx_ptr = &mut ctx_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;
    let mut error_out: *mut std::ffi::c_char = std::ptr::null_mut();
    let rc = unsafe { slot(ctx_ptr, &mut error_out) };
    if rc == 0 {
        return Ok(());
    }
    if error_out.is_null() {
        return Err(RuntimeError::Udf(format!(
            "UDF {slot_name} returned error code {rc}"
        )));
    }
    let text = unsafe { crate::single_call::take_c_string(error_out) };
    Err(RuntimeError::Udf(format!(
        "UDF {slot_name} returned error code {rc}: {text}"
    )))
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
#[path = "loader_tests.rs"]
mod tests;
