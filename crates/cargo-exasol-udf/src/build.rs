use std::ffi::CStr;
use std::path::Path;
use std::process::Command;

const MUSL_TARGET: &str = "x86_64-unknown-linux-musl";

/// Build the UDF crate at `path` for the musl target.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args.first().map(|s| s.as_str()).unwrap_or(".");
    let crate_dir = Path::new(path);
    let cargo_toml = crate_dir.join("Cargo.toml");

    if !cargo_toml.exists() {
        return Err(format!(
            "Cargo.toml not found in '{}' — is this a Rust crate?",
            path
        ));
    }

    // Parse crate name from Cargo.toml
    let crate_name = parse_crate_name(&cargo_toml)?;

    // Ensure the musl target is installed
    ensure_musl_target()?;

    // Run cargo build
    let status = Command::new("cargo")
        .args(["build", "--release", "--target", MUSL_TARGET])
        .current_dir(crate_dir)
        .status()
        .map_err(|e| format!("failed to run cargo: {}", e))?;

    if !status.success() {
        return Err(format!("cargo build failed with status: {}", status));
    }

    // Print the .so path
    let so_name = format!("lib{}.so", crate_name.replace('-', "_"));
    let so_path = crate_dir
        .join("target")
        .join(MUSL_TARGET)
        .join("release")
        .join(&so_name);

    println!("{}", so_path.display());

    // Try to emit schema sidecar if annotated schemas are present
    if so_path.exists()
        && let Err(e) = maybe_emit_sidecar(&so_path, &crate_name)
    {
        eprintln!("warning: could not emit schema sidecar: {}", e);
    }

    Ok(())
}

/// Parse `name = "..."` from the `[package]` section of Cargo.toml.
fn parse_crate_name(cargo_toml: &Path) -> Result<String, String> {
    let contents = std::fs::read_to_string(cargo_toml)
        .map_err(|e| format!("cannot read '{}': {}", cargo_toml.display(), e))?;

    let mut in_package = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if in_package && trimmed.starts_with('[') {
            // Left the [package] section
            break;
        }
        if in_package
            && trimmed.starts_with("name")
            && let Some(value) = trimmed
                .split_once('=')
                .map(|x| x.1.trim().trim_matches('"'))
        {
            return Ok(value.to_string());
        }
    }

    Err(format!(
        "could not find `name` in [package] section of '{}'",
        cargo_toml.display()
    ))
}

/// Ensure `x86_64-unknown-linux-musl` target is installed, adding it if missing.
fn ensure_musl_target() -> Result<(), String> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map_err(|e| format!("failed to run rustup: {}", e))?;

    let installed = String::from_utf8_lossy(&output.stdout);
    if installed.lines().any(|l| l.trim() == MUSL_TARGET) {
        return Ok(());
    }

    eprintln!("Installing target {}...", MUSL_TARGET);
    let status = Command::new("rustup")
        .args(["target", "add", MUSL_TARGET])
        .status()
        .map_err(|e| format!("failed to run rustup target add: {}", e))?;

    if !status.success() {
        return Err(format!("rustup target add {} failed", MUSL_TARGET));
    }

    Ok(())
}

/// Attempt to dlopen the `.so` and emit a `<name>.udf-meta.json` sidecar
/// if the vtable has non-null annotated schema pointers.
fn maybe_emit_sidecar(so_path: &Path, crate_name: &str) -> Result<(), String> {
    use libloading::Library;

    #[repr(C)]
    struct VTableProbe {
        abi_version: u32,
        fingerprint: *const std::ffi::c_char,
        // Skip run (fn pointer — 8 bytes on 64-bit)
        run: *const std::ffi::c_void,
        // Skip destroy (fn pointer)
        destroy: *const std::ffi::c_void,
        // Skip 4 optional fn pointers (each 8 bytes on 64-bit, as Option<fn> = 8)
        default_output_columns: *const std::ffi::c_void,
        virtual_schema_adapter_call: *const std::ffi::c_void,
        generate_sql_for_import_spec: *const std::ffi::c_void,
        generate_sql_for_export_spec: *const std::ffi::c_void,
        annotated_input_schema: *const std::ffi::c_char,
        annotated_output_schema: *const std::ffi::c_char,
    }

    let lib = unsafe { Library::new(so_path) }.map_err(|e| format!("dlopen failed: {}", e))?;

    let entry: libloading::Symbol<unsafe extern "C" fn() -> *const VTableProbe> =
        unsafe { lib.get(b"__exa_udf_entry\0") }
            .map_err(|_| "symbol __exa_udf_entry not found".to_string())?;

    let vtable = unsafe { entry() };
    if vtable.is_null() {
        return Err("__exa_udf_entry returned null".to_string());
    }

    let input_schema = unsafe { (*vtable).annotated_input_schema };
    let output_schema = unsafe { (*vtable).annotated_output_schema };

    if input_schema.is_null() || output_schema.is_null() {
        // Not annotated — no sidecar needed
        return Ok(());
    }

    let input_str = unsafe { CStr::from_ptr(input_schema) }
        .to_str()
        .map_err(|e| format!("input schema is not valid UTF-8: {}", e))?;
    let output_str = unsafe { CStr::from_ptr(output_schema) }
        .to_str()
        .map_err(|e| format!("output schema is not valid UTF-8: {}", e))?;

    // Parse to validate JSON, then emit
    let sidecar = format!(
        "{{\n  \"input_schema\": {},\n  \"output_schema\": {}\n}}\n",
        input_str, output_str
    );

    let sidecar_path = so_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{}.udf-meta.json", crate_name));

    std::fs::write(&sidecar_path, sidecar)
        .map_err(|e| format!("cannot write sidecar '{}': {}", sidecar_path.display(), e))?;

    println!("Schema sidecar: {}", sidecar_path.display());
    Ok(())
}
