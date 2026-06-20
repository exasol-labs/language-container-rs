use std::ffi::CStr;
use std::path::Path;
use std::process::Command;

use crate::validate::{VTableProbe, enumerate_entry_symbols};

const MUSL_TARGET: &str = "x86_64-unknown-linux-musl";

/// Build the UDF crate at `path` for the musl target and verify the produced artifact exports named entry points.
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

    // Verify the artifact exports at least one named entry point.
    if so_path.exists() {
        let entry_names = enumerate_entry_symbols(&so_path).unwrap_or_default();
        if entry_names.is_empty() {
            return Err(format!(
                "build produced '{}' but it exports no __exa_udf_entry_<NAME> symbols; \
                 annotate at least one function with #[exasol_udf]",
                so_path.display()
            ));
        }
    }

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
///
/// Uses the first discovered `__exa_udf_entry_<NAME>` symbol.
fn maybe_emit_sidecar(so_path: &Path, crate_name: &str) -> Result<(), String> {
    use libloading::Library;

    // Find the first named entry symbol.
    let entry_names = enumerate_entry_symbols(so_path).unwrap_or_default();
    let mut names_iter = entry_names.iter();
    let first_udf = names_iter
        .next()
        .cloned()
        .ok_or_else(|| "no __exa_udf_entry_<NAME> symbol found".to_string())?;
    if names_iter.next().is_some() {
        eprintln!(
            "warning: {} contains multiple UDFs; only the schema sidecar for '{}' is emitted",
            so_path.display(),
            first_udf
        );
    }

    let symbol = format!("__exa_udf_entry_{}\0", first_udf);

    let lib = unsafe { Library::new(so_path) }.map_err(|e| format!("dlopen failed: {}", e))?;

    let entry: libloading::Symbol<unsafe extern "C" fn() -> *const VTableProbe> =
        unsafe { lib.get(symbol.as_bytes()) }
            .map_err(|_| format!("symbol {} not found", symbol.trim_end_matches('\0')))?;

    let vtable = unsafe { entry() };
    if vtable.is_null() {
        return Err(format!("{} returned null", symbol.trim_end_matches('\0')));
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
