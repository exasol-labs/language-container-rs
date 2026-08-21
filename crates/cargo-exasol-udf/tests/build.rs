use std::path::{Path, PathBuf};
use std::process::Command;

fn cargo_exasol_udf_bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    loop {
        p.pop();
        if p.ends_with("debug") || p.ends_with("release") {
            break;
        }
        if p.parent().is_none() {
            panic!("Could not find target dir");
        }
    }
    p.push("cargo-exasol-udf");
    p
}

/// Scaffold a minimal cdylib crate in `dir` using cargo-exasol-udf new,
/// then return the path to it.
fn scaffold_udf_crate(parent: &Path, name: &str) -> PathBuf {
    let udf_path = parent.join(name);
    let status = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "new", udf_path.to_str().unwrap()])
        .status()
        .expect("cargo-exasol-udf new failed");
    assert!(status.success(), "scaffold failed");
    udf_path
}

/// Point the scaffold's crates.io SDK/macros deps at the in-repo workspace
/// crates so the build resolves against the LOCAL SDK, with no dependency on a
/// published crates.io version. Paths are canonicalized to absolute form —
/// a relative path written into the tempdir Cargo.toml would resolve against
/// the tempdir, not the repo.
fn patch_scaffold_to_local_sdk(udf_path: &Path) {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let sdk = workspace_root
        .join("crates/exasol-udf-sdk")
        .canonicalize()
        .expect("exasol-udf-sdk crate path");
    let macros = workspace_root
        .join("crates/exasol-udf-macros")
        .canonicalize()
        .expect("exasol-udf-macros crate path");

    let cargo_toml = udf_path.join("Cargo.toml");
    let mut contents = std::fs::read_to_string(&cargo_toml).unwrap();
    contents.push_str(&format!(
        "\n[patch.crates-io]\nexasol-udf-sdk = {{ path = \"{}\" }}\nexasol-udf-macros = {{ path = \"{}\" }}\n",
        sdk.display(),
        macros.display(),
    ));
    std::fs::write(&cargo_toml, contents).unwrap();
}

/// Read the host target triple from `rustc -vV` (the `host:` line). The host
/// target is installed by definition, so a native `--target <host>` build needs
/// no `rustup target add`.
fn host_target_triple() -> String {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("failed to run rustc -vV");
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("rustc -vV has no host: line")
        .trim()
        .to_string()
}

/// Set an explicit `[lib] name` on the scaffold, diverging the cdylib output
/// filename from the package name. `build` must honor it when computing the
/// artifact path.
fn set_lib_name(udf_path: &Path, lib_name: &str) {
    let cargo_toml = udf_path.join("Cargo.toml");
    let contents = std::fs::read_to_string(&cargo_toml).unwrap();
    let patched = contents.replacen(
        "[lib]\ncrate-type = [\"cdylib\"]",
        &format!("[lib]\nname = \"{lib_name}\"\ncrate-type = [\"cdylib\"]"),
        1,
    );
    std::fs::write(&cargo_toml, patched).unwrap();
}

/// Drop the `cdylib` crate-type so `cargo build --release` still succeeds but
/// produces no `.so` — reproducing the case where the artifact is missing at
/// the path `build` computed.
fn drop_cdylib_crate_type(udf_path: &Path) {
    let cargo_toml = udf_path.join("Cargo.toml");
    let contents = std::fs::read_to_string(&cargo_toml).unwrap();
    let patched = contents.replacen("crate-type = [\"cdylib\"]", "crate-type = [\"rlib\"]", 1);
    std::fs::write(&cargo_toml, patched).unwrap();
}

/// Enumerate exported `__exa_udf_entry_<NAME>` symbols in the built `.so` by
/// invoking `cargo-exasol-udf exasol-udf validate` and parsing the `<NAME>`s
/// it reports as OK from its stdout.
fn entry_symbols(so_path: &Path) -> Vec<String> {
    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf validate");
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|line| line.split_once(": ABI version"))
        .map(|(name, _)| name.to_string())
        .collect()
}

#[test]
fn build_produces_host_cdylib() {
    let dir = tempfile::tempdir().unwrap();
    let udf_path = scaffold_udf_crate(dir.path(), "test-build-udf");
    patch_scaffold_to_local_sdk(&udf_path);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "build", udf_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf build");

    assert!(
        output.status.success(),
        "build should succeed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("target/release/libtest_build_udf.so"),
        "stdout should print the host cdylib path: {stdout}"
    );
    assert!(
        !stdout.contains("x86_64-unknown-linux-musl"),
        "stdout must not reference the musl triple: {stdout}"
    );

    let so_path = udf_path.join("target/release/libtest_build_udf.so");
    assert!(
        so_path.exists(),
        "built .so should exist at {}",
        so_path.display()
    );

    let entries = entry_symbols(&so_path);
    assert!(
        !entries.is_empty(),
        "built .so should export at least one __exa_udf_entry_<NAME> symbol, found none"
    );
}

#[test]
fn build_honors_target_override() {
    let host = host_target_triple();
    let dir = tempfile::tempdir().unwrap();
    let udf_path = scaffold_udf_crate(dir.path(), "test-target-udf");
    patch_scaffold_to_local_sdk(&udf_path);

    let output = Command::new(cargo_exasol_udf_bin())
        .args([
            "exasol-udf",
            "build",
            udf_path.to_str().unwrap(),
            "--target",
            &host,
        ])
        .output()
        .expect("failed to run cargo-exasol-udf build");

    assert!(
        output.status.success(),
        "build --target {host} should succeed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = format!("target/{host}/release/libtest_target_udf.so");
    assert!(
        stdout.contains(&expected),
        "stdout should print the per-target cdylib path {expected}: {stdout}"
    );

    let so_path = udf_path.join(&expected);
    assert!(
        so_path.exists(),
        "built .so should exist at {}",
        so_path.display()
    );
}

#[test]
fn build_honors_explicit_lib_name() {
    let dir = tempfile::tempdir().unwrap();
    let udf_path = scaffold_udf_crate(dir.path(), "test-libname-udf");
    patch_scaffold_to_local_sdk(&udf_path);
    set_lib_name(&udf_path, "renamed_output");

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "build", udf_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf build");

    assert!(
        output.status.success(),
        "build should honor [lib] name and succeed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("target/release/librenamed_output.so"),
        "stdout should print the [lib] name-derived cdylib path: {stdout}"
    );

    let so_path = udf_path.join("target/release/librenamed_output.so");
    assert!(
        so_path.exists(),
        "built .so should exist at {}",
        so_path.display()
    );
}

#[test]
fn build_fails_when_artifact_missing_at_expected_path() {
    let dir = tempfile::tempdir().unwrap();
    let udf_path = scaffold_udf_crate(dir.path(), "test-missing-artifact-udf");
    patch_scaffold_to_local_sdk(&udf_path);
    drop_cdylib_crate_type(&udf_path);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "build", udf_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf build");

    assert!(
        !output.status.success(),
        "build must fail when cargo succeeds but no artifact exists at the expected path:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no artifact was produced"),
        "error should explain the missing artifact: {stderr}"
    );
}

#[test]
fn build_fails_on_missing_cargo_toml() {
    let dir = tempfile::tempdir().unwrap();
    let empty_path = dir.path().join("not-a-crate");
    std::fs::create_dir_all(&empty_path).unwrap();

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "build", empty_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf build");

    assert!(
        !output.status.success(),
        "build should fail when Cargo.toml is missing"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cargo.toml"),
        "error must mention Cargo.toml: {stderr}"
    );
}
