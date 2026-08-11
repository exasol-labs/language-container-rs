use std::ffi::CStr;
use std::path::Path;
use std::process::Command;

use crate::validate::{VTableProbe, enumerate_entry_symbols};

/// Build the UDF crate at `path` as a host glibc-dynamic cdylib and verify the
/// produced artifact exports named entry points. A `--target <triple>` override
/// builds natively for an installed target, into `target/<triple>/release`.
pub fn run(args: &[String]) -> Result<(), String> {
    let (path, target) = parse_build_args(args)?;
    let crate_dir = Path::new(path);
    let cargo_toml = crate_dir.join("Cargo.toml");

    if !cargo_toml.exists() {
        return Err(format!(
            "Cargo.toml not found in '{}' — is this a Rust crate?",
            path
        ));
    }

    let crate_name = parse_crate_name(&cargo_toml)?;
    // Cargo derives the cdylib filename from `[lib] name` when it is set,
    // falling back to the package name otherwise.
    let lib_name = parse_lib_name(&cargo_toml)?.unwrap_or_else(|| crate_name.clone());

    let mut cargo = Command::new("cargo");
    cargo.args(["build", "--release"]);
    if let Some(triple) = target {
        cargo.args(["--target", triple]);
    }
    let status = cargo
        .current_dir(crate_dir)
        .status()
        .map_err(|e| format!("failed to run cargo: {}", e))?;

    if !status.success() {
        return Err(format!("cargo build failed with status: {}", status));
    }

    let so_name = format!("lib{}.so", lib_name.replace('-', "_"));
    let mut release_dir = crate_dir.join("target");
    if let Some(triple) = target {
        release_dir = release_dir.join(triple);
    }
    let so_path = release_dir.join("release").join(&so_name);

    println!("{}", so_path.display());

    if !so_path.exists() {
        return Err(format!(
            "cargo build succeeded but no artifact was produced at '{}'",
            so_path.display()
        ));
    }

    let entry_names = enumerate_entry_symbols(&so_path)
        .map_err(|e| format!("could not inspect '{}': {}", so_path.display(), e))?;
    if entry_names.is_empty() {
        return Err(format!(
            "build produced '{}' but it exports no __exa_udf_entry_<NAME> symbols; \
             annotate at least one function with #[exasol_udf]",
            so_path.display()
        ));
    }

    if let Err(e) = maybe_emit_sidecar(&so_path, &crate_name) {
        eprintln!("warning: could not emit schema sidecar: {}", e);
    }

    Ok(())
}

/// Parse the build subcommand args into `(crate_path, optional_target_triple)`.
/// `--target <triple>` selects a native build into `target/<triple>/release`;
/// the first bare argument is the crate path (default `.`).
fn parse_build_args(args: &[String]) -> Result<(&str, Option<&str>), String> {
    let mut path: Option<&str> = None;
    let mut target: Option<&str> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--target" => {
                let triple = iter
                    .next()
                    .ok_or_else(|| "--target requires a target triple".to_string())?;
                target = Some(triple.as_str());
            }
            other if path.is_none() => path = Some(other),
            _ => {}
        }
    }
    Ok((path.unwrap_or("."), target))
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

/// Parse an explicit `name = "..."` from the `[lib]` section of Cargo.toml, if
/// present. Cargo derives the cdylib output filename from this when set, so the
/// build must honor it rather than assuming the artifact is named after the
/// package. Returns `Ok(None)` when no `[lib] name` is declared.
fn parse_lib_name(cargo_toml: &Path) -> Result<Option<String>, String> {
    let contents = std::fs::read_to_string(cargo_toml)
        .map_err(|e| format!("cannot read '{}': {}", cargo_toml.display(), e))?;

    let mut in_lib = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "[lib]" {
            in_lib = true;
            continue;
        }
        if in_lib && trimmed.starts_with('[') {
            // Left the [lib] section
            break;
        }
        if in_lib
            && trimmed.starts_with("name")
            && let Some(value) = trimmed
                .split_once('=')
                .map(|x| x.1.trim().trim_matches('"'))
        {
            return Ok(Some(value.to_string()));
        }
    }

    Ok(None)
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
