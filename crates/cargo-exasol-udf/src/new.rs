use std::fs;
use std::path::Path;

/// Scaffold a new UDF crate at `path`.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or_else(|| "Usage: cargo exasol-udf new <path>".to_string())?;
    let target = Path::new(path);

    if target.exists() {
        // Non-empty check: any entry in the directory is a rejection
        let has_entries = fs::read_dir(target)
            .map_err(|e| format!("cannot read directory '{}': {}", path, e))?
            .next()
            .is_some();
        if has_entries {
            return Err(format!("target directory '{}' is not empty", path));
        }
    }

    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("invalid path: {}", path))?;

    // Create directory structure
    let src_dir = target.join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|e| format!("cannot create directory '{}': {}", src_dir.display(), e))?;

    // Pin the SDK/macros to this tool's own version so a fresh scaffold always
    // targets the SLC it was released alongside — the ABI fingerprint bakes in
    // the SDK version, so a stale hardcoded pin would silently be a release
    // behind. cargo-exasol-udf shares the workspace version with the SDK.
    let sdk_version = env!("CARGO_PKG_VERSION");

    // Write Cargo.toml
    let cargo_toml = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
exasol-udf-sdk = {{ version = "{sdk_version}", features = [] }}
exasol-udf-macros = {{ version = "{sdk_version}" }}

[profile.release]
strip = true
"#
    );
    let cargo_path = target.join("Cargo.toml");
    fs::write(&cargo_path, cargo_toml)
        .map_err(|e| format!("cannot write '{}': {}", cargo_path.display(), e))?;

    // Write src/lib.rs
    let lib_rs = r#"use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

// The SQL script name maps to this function's name: `run` exports the entry
// symbol `__exa_udf_entry_RUN`, so register it with `CREATE ... SCRIPT run ...`.
// Rename this fn — or add `#[exasol_udf(name = "MY_NAME")]` — to change it.
#[exasol_udf]
fn run(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    // TODO: implement your UDF
    Ok(None)
}
"#;
    let lib_path = src_dir.join("lib.rs");
    fs::write(&lib_path, lib_rs)
        .map_err(|e| format!("cannot write '{}': {}", lib_path.display(), e))?;

    println!("Created UDF crate at {}", path);
    Ok(())
}
