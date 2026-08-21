use std::ffi::CStr;
use std::path::Path;

use crate::elf;
use crate::slc_surface::{self, FloorCompliance};

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

/// Validate a compiled UDF `.so`: checks that it exports named entry points with matching ABI version and SDK fingerprint, that it references no glibc symbol version above the SLC's floor, and reports its dynamic dependencies against the SLC library surface.
pub fn run(args: &[String]) -> Result<(), String> {
    let (path, deny_unknown_deps) = parse_validate_args(args)?;
    let so_path = Path::new(path);

    let artifact = elf::read(so_path).map_err(|error| error.to_string())?;

    if artifact.udf_names.is_empty() {
        return Err(format!(
            "no __exa_udf_entry_<NAME> entry point found in '{}'; \
             hint: rebuild against sdk >= 0.14.0",
            so_path.display()
        ));
    }

    report_glibc_floor(&artifact, so_path)?;
    report_dynamic_dependencies(&artifact, so_path, deny_unknown_deps)?;
    verify_vtables(&artifact, so_path)?;

    println!(
        "✓ {} UDF(s) validated in '{}'",
        artifact.udf_names.len(),
        so_path.display()
    );
    Ok(())
}

/// Report the artifact's highest referenced `GLIBC_x.y` symbol version against
/// the SLC's committed floor, failing if it exceeds what the container ships.
fn report_glibc_floor(artifact: &elf::SharedObject, so_path: &Path) -> Result<(), String> {
    let floor = slc_surface::glibc_floor();
    match &artifact.max_glibc_version {
        Some(version) => {
            if slc_surface::check_against_floor(version) == FloorCompliance::ExceedsFloor {
                return Err(format!(
                    "'{}' references GLIBC_{version} which exceeds the SLC's glibc floor {floor}; \
                     it cannot load in the container",
                    so_path.display()
                ));
            }
            println!("glibc: highest reference GLIBC_{version} (SLC floor {floor})");
        }
        None => println!("glibc: no GLIBC_x.y reference (SLC floor {floor})"),
    }
    Ok(())
}

/// Report the artifact's `DT_NEEDED` dependencies against the SLC library
/// surface, escalating an unstaged dependency to a hard failure when
/// `deny_unknown_deps` is set.
fn report_dynamic_dependencies(
    artifact: &elf::SharedObject,
    so_path: &Path,
    deny_unknown_deps: bool,
) -> Result<(), String> {
    let unknown_deps =
        slc_surface::unknown_sonames(artifact.needed_sonames.iter().map(String::as_str));
    if unknown_deps.is_empty() {
        println!(
            "dependencies: {} (all within the SLC library surface)",
            artifact.needed_sonames.join(", ")
        );
    } else if deny_unknown_deps {
        return Err(format!(
            "'{}' links dependencies outside the SLC library surface: {}",
            so_path.display(),
            unknown_deps.join(", ")
        ));
    } else {
        println!(
            "warning: '{}' links dependencies outside the SLC library surface: {}",
            so_path.display(),
            unknown_deps.join(", ")
        );
    }
    Ok(())
}

/// dlopen the artifact and probe every named entry point's vtable, failing if
/// any entry's ABI version or SDK fingerprint does not match this runtime.
fn verify_vtables(artifact: &elf::SharedObject, so_path: &Path) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();
    let mut ok_names: Vec<String> = Vec::new();
    let rt_fingerprint = runtime_fingerprint();

    for udf_name in &artifact.udf_names {
        let symbol = format!("__exa_udf_entry_{}\0", udf_name);
        match load_vtable_fields(so_path, symbol.as_bytes()) {
            Err(e) => errors.push(format!("  {}: {}", udf_name, e)),
            Ok((abi_version, fingerprint)) => {
                if abi_version != RUNTIME_ABI_VERSION {
                    errors.push(format!(
                        "  {}: ABI version mismatch — .so has {}, runtime expects {}",
                        udf_name, abi_version, RUNTIME_ABI_VERSION
                    ));
                } else if fingerprint != rt_fingerprint {
                    errors.push(format!(
                        "  {}: SDK fingerprint mismatch — .so has '{}', runtime has '{}'",
                        udf_name, fingerprint, rt_fingerprint
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
    Ok(())
}

/// Parse the validate subcommand args into `(so_path, deny_unknown_deps)`.
/// `--deny-unknown-deps` escalates an unstaged `DT_NEEDED` entry from a warning
/// to a hard failure; the first bare argument is the artifact path.
fn parse_validate_args(args: &[String]) -> Result<(&str, bool), String> {
    let mut path: Option<&str> = None;
    let mut deny_unknown_deps = false;
    for arg in args {
        match arg.as_str() {
            "--deny-unknown-deps" => deny_unknown_deps = true,
            other if other.starts_with("--") => {
                return Err(format!(
                    "unrecognized argument '{other}'. Usage: cargo exasol-udf validate <path-to-so> [--deny-unknown-deps]"
                ));
            }
            other if path.is_none() => path = Some(other),
            other => {
                return Err(format!(
                    "unrecognized argument '{other}'. Usage: cargo exasol-udf validate <path-to-so> [--deny-unknown-deps]"
                ));
            }
        }
    }
    let path = path.ok_or_else(|| {
        "Usage: cargo exasol-udf validate <path-to-so> [--deny-unknown-deps]".to_string()
    })?;
    Ok((path, deny_unknown_deps))
}

/// Enumerate all exported `__exa_udf_entry_<NAME>` symbols in the `.so`,
/// returning the `<NAME>` suffixes (e.g. `["DOUBLE_IT", "TRIPLE_IT"]`).
///
/// Reads the artifact in-process via [`elf`], so an input that is not a
/// parseable ELF shared object fails here rather than passing on as an
/// artifact that merely exports no entry points.
pub(crate) fn enumerate_entry_symbols(so_path: &Path) -> Result<Vec<String>, String> {
    elf::read(so_path)
        .map(|artifact| artifact.udf_names)
        .map_err(|error| error.to_string())
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
