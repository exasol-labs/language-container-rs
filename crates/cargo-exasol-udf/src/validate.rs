use std::ffi::CStr;
use std::path::Path;
use std::process::Command;

/// ABI version this binary was compiled against.
const RUNTIME_ABI_VERSION: u32 = exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION;

/// SDK fingerprint this binary expects. Sourced from the linked `exasol-udf-sdk`
/// (the single source of truth — the same constant the macro bakes into every
/// `.so` and the runtime checks at load), with the trailing C NUL stripped so it
/// compares equal to the `CStr`-decoded fingerprint read from the `.so`.
fn runtime_fingerprint() -> &'static str {
    exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT.trim_end_matches('\0')
}

/// A `#[repr(C)]` mirror of `ExaUdfVTable` used to probe ABI fields without
/// linking against the UDF `.so`'s SDK. Field names and byte-offset comments
/// match the canonical `ExaUdfVTable` in `exasol-udf-sdk`.
///
/// Using the full 10-field layout (rather than a 2-field truncation) ensures
/// that any future field added before `annotated_input_schema` is caught at
/// review time rather than silently misaligning the sidecar path in `build.rs`.
///
/// Callers that only need `abi_version` / `fingerprint` simply ignore the rest.
#[repr(C)]
pub(crate) struct VTableProbe {
    /// Offset 0: ABI version baked into the vtable at compile time.
    pub(crate) abi_version: u32,
    /// Offset 8 (4 bytes + 4 pad): null-terminated fingerprint in .rodata.
    pub(crate) fingerprint: *const std::ffi::c_char,
    /// Offset 16: run fn pointer (8 bytes).
    pub(crate) run: *const std::ffi::c_void,
    /// Offset 24: destroy fn pointer (8 bytes).
    pub(crate) destroy: *const std::ffi::c_void,
    /// Offset 32: optional default_output_columns fn (Option<fn> = 8 bytes).
    pub(crate) default_output_columns: *const std::ffi::c_void,
    /// Offset 40: optional virtual_schema_adapter_call fn (8 bytes).
    pub(crate) virtual_schema_adapter_call: *const std::ffi::c_void,
    /// Offset 48: optional generate_sql_for_import_spec fn (8 bytes).
    pub(crate) generate_sql_for_import_spec: *const std::ffi::c_void,
    /// Offset 56: optional generate_sql_for_export_spec fn (8 bytes).
    pub(crate) generate_sql_for_export_spec: *const std::ffi::c_void,
    /// Offset 64: nullable pointer to annotated input schema JSON.
    pub(crate) annotated_input_schema: *const std::ffi::c_char,
    /// Offset 72: nullable pointer to annotated output schema JSON.
    pub(crate) annotated_output_schema: *const std::ffi::c_char,
}

/// Validate a compiled UDF `.so`: checks that it exports named entry points with matching ABI version and SDK fingerprint.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or_else(|| "Usage: cargo exasol-udf validate <path-to-so>".to_string())?;
    let so_path = Path::new(path);

    if !so_path.exists() {
        return Err(format!("file not found: '{}'", path));
    }

    let udf_names = enumerate_entry_symbols(so_path)?;

    if udf_names.is_empty() {
        return Err(format!(
            "no __exa_udf_entry_<NAME> entry point found in '{}'; \
             hint: rebuild against sdk >= 0.14.0",
            so_path.display()
        ));
    }

    let mut errors: Vec<String> = Vec::new();
    let mut ok_names: Vec<String> = Vec::new();

    for udf_name in &udf_names {
        let symbol = format!("__exa_udf_entry_{}\0", udf_name);
        match load_vtable_fields(so_path, symbol.as_bytes()) {
            Err(e) => errors.push(format!("  {}: {}", udf_name, e)),
            Ok((abi_version, fingerprint)) => {
                if abi_version != RUNTIME_ABI_VERSION {
                    errors.push(format!(
                        "  {}: ABI version mismatch — .so has {}, runtime expects {}",
                        udf_name, abi_version, RUNTIME_ABI_VERSION
                    ));
                } else if fingerprint != runtime_fingerprint() {
                    errors.push(format!(
                        "  {}: SDK fingerprint mismatch — .so has '{}', runtime has '{}'",
                        udf_name,
                        fingerprint,
                        runtime_fingerprint()
                    ));
                } else {
                    ok_names.push(udf_name.clone());
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(format!(
            "validation failed for '{}':\n{}",
            so_path.display(),
            errors.join("\n")
        ));
    }

    for name in &ok_names {
        println!(
            "  {}: ABI version {}, fingerprint {} — OK",
            name,
            RUNTIME_ABI_VERSION,
            runtime_fingerprint()
        );
    }
    println!(
        "✓ {} UDF(s) validated in '{}'",
        ok_names.len(),
        so_path.display()
    );
    Ok(())
}

/// Enumerate all exported `__exa_udf_entry_<NAME>` symbols in the `.so`.
///
/// Uses `nm --dynamic --defined-only` to read the dynamic symbol table without
/// dlopening the library — no new crate dependency required.
/// Returns the `<NAME>` suffixes (e.g. `["DOUBLE_IT", "TRIPLE_IT"]`).
pub(crate) fn enumerate_entry_symbols(so_path: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("nm")
        .arg("--dynamic")
        .arg("--defined-only")
        .arg(so_path)
        .output()
        .map_err(|e| format!("failed to run `nm` (install binutils and retry): {e}"))?;

    if !output.status.success() {
        // nm can fail for non-ELF files — treat as zero entry points, not an error.
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let prefix = "__exa_udf_entry_";
    let names: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            // nm output: "<addr> <type> <name>" — name is the last whitespace-delimited field.
            let sym = line.split_whitespace().last()?;
            sym.strip_prefix(prefix).map(|s| s.to_string())
        })
        .collect();

    Ok(names)
}

/// dlopen the `.so`, resolve the named entry symbol, and return `(abi_version, fingerprint)`.
///
/// `symbol_bytes` must be a NUL-terminated byte sequence, e.g. `b"__exa_udf_entry_FOO\0"`.
fn load_vtable_fields(so_path: &Path, symbol_bytes: &[u8]) -> Result<(u32, String), String> {
    use libloading::Library;

    let lib = unsafe { Library::new(so_path) }
        .map_err(|e| format!("dlopen '{}' failed: {}", so_path.display(), e))?;

    let entry: libloading::Symbol<unsafe extern "C" fn() -> *const VTableProbe> =
        unsafe { lib.get(symbol_bytes) }.map_err(|_| {
            let sym_name = std::str::from_utf8(symbol_bytes)
                .unwrap_or("<invalid>")
                .trim_end_matches('\0');
            format!("symbol {} not found in '{}'", sym_name, so_path.display())
        })?;

    let vtable = unsafe { entry() };
    if vtable.is_null() {
        return Err("entry function returned null vtable pointer".to_string());
    }

    let abi_version = unsafe { (*vtable).abi_version };
    let fingerprint_ptr = unsafe { (*vtable).fingerprint };
    if fingerprint_ptr.is_null() {
        return Err("vtable fingerprint pointer is null".to_string());
    }

    let fingerprint = unsafe { CStr::from_ptr(fingerprint_ptr) }
        .to_str()
        .map_err(|e| format!("fingerprint is not valid UTF-8: {}", e))?
        .to_string();

    // Keep the library alive until we've read the data.
    // After drop, the vtable's .rodata is unmapped — but we've copied both values above.
    drop(lib);

    Ok((abi_version, fingerprint))
}
