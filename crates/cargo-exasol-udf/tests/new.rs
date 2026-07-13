use std::fs;
use std::process::Command;

fn cargo_exasol_udf_bin() -> std::path::PathBuf {
    // Use the already-built binary from target/debug
    let mut p = std::env::current_exe().unwrap();
    // walk up to the target/debug dir
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

#[test]
fn new_scaffolds_crate_files() {
    let dir = tempfile::tempdir().unwrap();
    let udf_path = dir.path().join("my-udf");

    let status = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "new", udf_path.to_str().unwrap()])
        .status()
        .expect("failed to run cargo-exasol-udf");

    assert!(status.success(), "cargo-exasol-udf new should succeed");

    // Cargo.toml must exist and contain cdylib
    let cargo_toml = udf_path.join("Cargo.toml");
    assert!(cargo_toml.exists(), "Cargo.toml must be created");
    let contents = fs::read_to_string(&cargo_toml).unwrap();
    assert!(
        contents.contains("cdylib"),
        "Cargo.toml must declare cdylib"
    );
    assert!(
        contents.contains("my-udf"),
        "Cargo.toml must use directory name"
    );
    assert!(
        contents.contains("exasol-udf-sdk"),
        "Cargo.toml must depend on exasol-udf-sdk"
    );

    // src/lib.rs must exist
    let lib_rs = udf_path.join("src").join("lib.rs");
    assert!(lib_rs.exists(), "src/lib.rs must be created");
    let lib_contents = fs::read_to_string(&lib_rs).unwrap();
    assert!(
        lib_contents.contains("exasol_udf"),
        "lib.rs must reference exasol_udf macro"
    );
    assert!(
        lib_contents.contains("Result<Option<i64>, UdfError>"),
        "lib.rs must model the scalar RETURNS channel"
    );
    assert!(
        lib_contents.contains("Ok(None)"),
        "lib.rs scaffold body must return Ok(None) for the RETURNS channel"
    );
}

#[test]
fn new_rejects_existing_nonempty_target() {
    let dir = tempfile::tempdir().unwrap();
    let udf_path = dir.path().join("my-udf");
    fs::create_dir_all(&udf_path).unwrap();
    // Put a file in it to make it non-empty
    fs::write(udf_path.join("existing_file.txt"), "data").unwrap();

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "new", udf_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "cargo-exasol-udf new should fail on non-empty target"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not empty") || stderr.contains("non-empty"),
        "error message should mention non-empty: {stderr}"
    );
}
