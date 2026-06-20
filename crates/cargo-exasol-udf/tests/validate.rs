use std::path::{Path, PathBuf};
use std::process::Command;

fn cargo_exasol_udf_bin() -> std::path::PathBuf {
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

/// Compile `source` as a cdylib into `out_dir/lib<name>.so` and return the path.
fn compile_fixture(out_dir: &Path, name: &str, source: &str) -> PathBuf {
    let src_path = out_dir.join(format!("{name}.rs"));
    std::fs::write(&src_path, source).expect("write fixture source");
    let so_path = out_dir.join(format!("lib{name}.so"));

    let status = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("-o")
        .arg(&so_path)
        .arg(&src_path)
        .status()
        .expect("invoke rustc");
    assert!(status.success(), "rustc failed to compile fixture {name}");
    so_path
}

/// Compute the same fingerprint the cargo-exasol-udf binary uses:
/// "0.1.1:<sanitized-rustc-version>".
fn compute_expected_fingerprint() -> String {
    let output = Command::new("rustc")
        .arg("--version")
        .output()
        .expect("rustc --version");
    let rustc_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let hash_part: String = rustc_hash
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    format!("0.1.1:{}", hash_part)
}

/// Generate a cdylib source that exports `__exa_udf_entry_<udf_name>` with the
/// given abi_version and fingerprint (must include a trailing `\0` in the string
/// literal for it to be a valid C string).
fn named_entry_fixture_source(
    abi_version: u32,
    fingerprint_with_nul: &str,
    udf_name: &str,
) -> String {
    format!(
        r#"
use std::ffi::c_void;
use std::os::raw::c_char;

#[repr(C)]
pub struct ExaUdfVTable {{
    pub abi_version: u32,
    pub fingerprint: *const c_char,
    pub run: unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32,
    pub destroy: unsafe extern "C" fn(),
}}

unsafe impl Sync for ExaUdfVTable {{}}

unsafe extern "C" fn run(_ctx: *mut c_void, _error_out: *mut *mut c_char) -> i32 {{ 0 }}
unsafe extern "C" fn destroy() {{}}

static FINGERPRINT: &str = "{fingerprint_with_nul}";

static VTABLE: ExaUdfVTable = ExaUdfVTable {{
    abi_version: {abi_version},
    fingerprint: FINGERPRINT.as_ptr() as *const c_char,
    run,
    destroy,
}};

#[no_mangle]
pub extern "C" fn __exa_udf_entry_{udf_name}() -> *const ExaUdfVTable {{
    &VTABLE as *const ExaUdfVTable
}}
"#
    )
}

/// Generate a cdylib source that exports TWO named entry points.
fn two_named_entries_fixture_source(abi_version: u32, fingerprint_with_nul: &str) -> String {
    format!(
        r#"
use std::ffi::c_void;
use std::os::raw::c_char;

#[repr(C)]
pub struct ExaUdfVTable {{
    pub abi_version: u32,
    pub fingerprint: *const c_char,
    pub run: unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32,
    pub destroy: unsafe extern "C" fn(),
}}

unsafe impl Sync for ExaUdfVTable {{}}

unsafe extern "C" fn run(_ctx: *mut c_void, _error_out: *mut *mut c_char) -> i32 {{ 0 }}
unsafe extern "C" fn destroy() {{}}

static FINGERPRINT: &str = "{fingerprint_with_nul}";

static VTABLE: ExaUdfVTable = ExaUdfVTable {{
    abi_version: {abi_version},
    fingerprint: FINGERPRINT.as_ptr() as *const c_char,
    run,
    destroy,
}};

#[no_mangle]
pub extern "C" fn __exa_udf_entry_DOUBLE_IT() -> *const ExaUdfVTable {{
    &VTABLE as *const ExaUdfVTable
}}

#[no_mangle]
pub extern "C" fn __exa_udf_entry_TRIPLE_IT() -> *const ExaUdfVTable {{
    &VTABLE as *const ExaUdfVTable
}}
"#
    )
}

/// Minimal tempdir without extra dependency.
fn tempdir() -> TempDir {
    let mut base = std::env::temp_dir();
    let unique = format!(
        "cargo-exaudf-validate-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    base.push(unique);
    std::fs::create_dir_all(&base).expect("create tempdir");
    TempDir { path: base }
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Existing tests (preserved)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn validate_rejects_missing_file() {
    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", "/nonexistent/path/lib.so"])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should fail for missing file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found")
            || stderr.contains("No such file")
            || stderr.contains("does not exist"),
        "error should mention missing file: {stderr}"
    );
}

#[test]
fn validate_rejects_missing_entry_symbol() {
    // A .so that exports NO __exa_udf_entry_* symbols — use a system lib as stand-in.
    let candidates = [
        "/usr/lib/x86_64-linux-gnu/libm.so.6",
        "/lib/x86_64-linux-gnu/libm.so.6",
        "/usr/lib/libm.so.6",
    ];
    let so_path = candidates.iter().find(|p| std::path::Path::new(p).exists());
    let so_path = match so_path {
        Some(p) => p,
        None => {
            eprintln!("SKIP: no system .so found for validate_rejects_missing_entry_symbol");
            return;
        }
    };

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so_path])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should fail when no __exa_udf_entry_* is present"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("__exa_udf_entry_")
            || stderr.contains("no entry")
            || stderr.contains("entry point"),
        "error should mention missing named entry: {stderr}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// New tests for named-entry enumeration
// ─────────────────────────────────────────────────────────────────────────────

/// validate accepts a .so that exports one or more __exa_udf_entry_<NAME> symbols
/// with matching abi_version and sdk_fingerprint.
#[test]
fn validate_accepts_named_entries() {
    let dir = tempdir();
    let fingerprint = compute_expected_fingerprint();
    // Fingerprint in the vtable is a C string, so append \0.
    let fingerprint_with_nul = format!("{}\0", fingerprint);
    // abi_version 4 must match what cargo-exasol-udf was compiled against.
    let src = two_named_entries_fixture_source(4, &fingerprint_with_nul);
    let so = compile_fixture(dir.path(), "two_named_entries", &src);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "validate should succeed for a .so with matching named entries\nstdout={stdout}\nstderr={stderr}"
    );
    // Should mention both discovered UDF names.
    assert!(
        stdout.contains("DOUBLE_IT") || stderr.contains("DOUBLE_IT"),
        "output should mention DOUBLE_IT\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("TRIPLE_IT") || stderr.contains("TRIPLE_IT"),
        "output should mention TRIPLE_IT\nstdout={stdout}\nstderr={stderr}"
    );
}

/// validate rejects a .so whose vtable has a wrong abi_version.
#[test]
fn validate_rejects_abi_mismatch() {
    let dir = tempdir();
    let fingerprint = compute_expected_fingerprint();
    let fingerprint_with_nul = format!("{}\0", fingerprint);
    // Use abi_version 99 — intentionally wrong.
    let src = named_entry_fixture_source(99, &fingerprint_with_nul, "MY_UDF");
    let so = compile_fixture(dir.path(), "abi_mismatch", &src);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should fail on abi_version mismatch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ABI") || stderr.contains("abi"),
        "error should mention ABI mismatch: {stderr}"
    );
    assert!(
        stderr.contains("MY_UDF"),
        "error should name the offending UDF: {stderr}"
    );
}

/// validate rejects a .so whose vtable has a wrong sdk_fingerprint.
#[test]
fn validate_rejects_fingerprint_mismatch() {
    let dir = tempdir();
    // Wrong fingerprint — deliberately does not match the binary's RUNTIME_FINGERPRINT.
    let src = named_entry_fixture_source(4, "0.0.0:definitely-wrong-fingerprint\0", "MY_UDF");
    let so = compile_fixture(dir.path(), "fp_mismatch", &src);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should fail on fingerprint mismatch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("fingerprint") || stderr.contains("SDK"),
        "error should mention fingerprint mismatch: {stderr}"
    );
    assert!(
        stderr.contains("MY_UDF"),
        "error should name the offending UDF: {stderr}"
    );
}

/// validate rejects a .so that exports zero __exa_udf_entry_* symbols (named),
/// even if it exports the legacy bare __exa_udf_entry.
#[test]
fn validate_rejects_no_named_entry_symbols() {
    let dir = tempdir();
    // A .so that only exports the OLD bare __exa_udf_entry — no named symbols.
    let src = r#"
use std::ffi::c_void;
use std::os::raw::c_char;

#[repr(C)]
pub struct ExaUdfVTable {
    pub abi_version: u32,
    pub fingerprint: *const c_char,
    pub run: unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32,
    pub destroy: unsafe extern "C" fn(),
}
unsafe impl Sync for ExaUdfVTable {}

unsafe extern "C" fn run(_: *mut c_void, _: *mut *mut c_char) -> i32 { 0 }
unsafe extern "C" fn destroy() {}

static FP: &str = "0.0.0:old\0";
static VTABLE: ExaUdfVTable = ExaUdfVTable {
    abi_version: 4,
    fingerprint: FP.as_ptr() as *const c_char,
    run,
    destroy,
};

#[no_mangle]
pub extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable {
    &VTABLE as *const ExaUdfVTable
}
"#;
    let so = compile_fixture(dir.path(), "legacy_bare_entry", src);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should fail for .so with no named __exa_udf_entry_* symbols"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("__exa_udf_entry_")
            || stderr.contains("no entry")
            || stderr.contains("entry point")
            || stderr.contains("rebuild"),
        "error should mention missing named entry or rebuild hint: {stderr}"
    );
}

/// Verifies that the validate subcommand (and thus `enumerate_entry_symbols`)
/// correctly reports zero named entries for a `.so` that exports no
/// `__exa_udf_entry_*` symbols — exercising the predicate that `build::run`
/// relies on when checking the produced artifact.
#[test]
fn build_verifies_named_entry() {
    let dir = tempdir();
    // A plain cdylib with one exported function but no __exa_udf_entry_* symbol.
    let src = r#"
#[no_mangle]
pub extern "C" fn just_a_plain_function() -> u32 { 42 }
"#;
    let so = compile_fixture(dir.path(), "no_entry_symbols", src);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should reject a .so with no __exa_udf_entry_* symbols"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("__exa_udf_entry_")
            || stderr.contains("no entry")
            || stderr.contains("entry point")
            || stderr.contains("rebuild"),
        "error should mention missing named entry: {stderr}"
    );
}
