//! Loader gating tests. These build tiny cdylib `.so` fixtures with `rustc`
//! at test time, each exporting `__exa_udf_entry` returning a hand-crafted
//! vtable, then assert the loader rejects ABI/fingerprint mismatches.

use std::path::{Path, PathBuf};
use std::process::Command;

use exa_udf_runtime::{LoadedUdf, RuntimeError};
use exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION;

/// Compile `source` as a cdylib into `out_dir/<name>.so` and return the path.
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

/// A fixture exporting a vtable with the given abi_version and fingerprint
/// (NUL-terminated C string). The run/destroy fns are no-ops.
fn fixture_source(abi_version: u32, fingerprint_with_nul: &str) -> String {
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
pub extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable {{
    &VTABLE as *const ExaUdfVTable
}}
"#
    )
}

#[test]
fn loader_rejects_abi_mismatch() {
    let dir = tempdir();
    // abi_version 99 (not 1). Fingerprint is irrelevant: the loader checks the
    // ABI version first and must bail before touching the fingerprint.
    let src = fixture_source(99, "0.0.0:wrong\\0");
    let so = compile_fixture(dir.path(), "abi_mismatch", &src);

    match LoadedUdf::open(&so) {
        Err(RuntimeError::AbiMismatch { expected, found }) => {
            assert_eq!(expected, EXA_UDF_ABI_VERSION);
            assert_eq!(found, 99);
        }
        Err(other) => panic!("expected AbiMismatch, got {other:?}"),
        Ok(_) => panic!("expected AbiMismatch, got a loaded UDF"),
    }
}

#[test]
fn loader_rejects_fingerprint_mismatch() {
    let dir = tempdir();
    // Correct abi_version so the loader proceeds to the fingerprint check, but
    // a deliberately wrong fingerprint body.
    let src = fixture_source(EXA_UDF_ABI_VERSION, "0.0.0:definitely-not-the-host\\0");
    let so = compile_fixture(dir.path(), "fp_mismatch", &src);

    match LoadedUdf::open(&so) {
        Err(RuntimeError::FingerprintMismatch { found, .. }) => {
            assert_eq!(found, "0.0.0:definitely-not-the-host");
        }
        Err(other) => panic!("expected FingerprintMismatch, got {other:?}"),
        Ok(_) => panic!("expected FingerprintMismatch, got a loaded UDF"),
    }
}

#[test]
fn loader_rejects_missing_entry_symbol() {
    let dir = tempdir();
    // A cdylib with no __exa_udf_entry symbol.
    let so = compile_fixture(
        dir.path(),
        "no_entry",
        "#[no_mangle] pub extern \"C\" fn unrelated() {}\n",
    );
    let result = LoadedUdf::open(&so);
    assert!(matches!(result, Err(RuntimeError::Loader(_))));
}

/// Minimal tempdir without pulling an extra dev-dependency.
fn tempdir() -> TempDir {
    let mut base = std::env::temp_dir();
    let unique = format!(
        "exa-udf-loader-test-{}-{}",
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
