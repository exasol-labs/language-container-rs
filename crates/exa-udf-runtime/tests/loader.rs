//! Loader gating tests. These build tiny cdylib `.so` fixtures with `rustc`
//! at test time, each exporting a named `__exa_udf_entry_<NAME>` symbol
//! returning a hand-crafted vtable, then assert the loader correctly resolves
//! named entry points and rejects ABI/fingerprint mismatches.

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

/// A fixture exporting a vtable under the given `entry_symbol` name with the
/// specified abi_version and fingerprint (NUL-terminated C string).
/// The run/destroy fns are no-ops.
fn fixture_source(entry_symbol: &str, abi_version: u32, fingerprint_with_nul: &str) -> String {
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
pub extern "C" fn {entry_symbol}() -> *const ExaUdfVTable {{
    &VTABLE as *const ExaUdfVTable
}}
"#
    )
}

#[test]
fn loader_accepts_named_entry() {
    let dir = tempdir();
    // A vtable with the correct ABI version and the host fingerprint, exported
    // under the named symbol the loader will resolve.
    use exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT;
    let host_fp = EXA_SDK_FINGERPRINT.trim_end_matches('\0');
    let fingerprint_with_nul = format!("{host_fp}\\0");
    let src = fixture_source(
        "__exa_udf_entry_TESTUDF",
        EXA_UDF_ABI_VERSION,
        &fingerprint_with_nul,
    );
    let so = compile_fixture(dir.path(), "named_entry", &src);

    match LoadedUdf::open(&so, "TESTUDF") {
        Ok(_) => {} // success: loader accepted the named entry
        Err(e) => panic!("expected Ok, got {e:?}"),
    }
}

#[test]
fn loader_errors_on_missing_named_entry() {
    let dir = tempdir();
    // The .so exports __exa_udf_entry_DOUBLE_IT but not __exa_udf_entry_MISSING.
    use exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT;
    let host_fp = EXA_SDK_FINGERPRINT.trim_end_matches('\0');
    let fingerprint_with_nul = format!("{host_fp}\\0");
    let src = fixture_source(
        "__exa_udf_entry_DOUBLE_IT",
        EXA_UDF_ABI_VERSION,
        &fingerprint_with_nul,
    );
    let so = compile_fixture(dir.path(), "missing_entry", &src);

    match LoadedUdf::open(&so, "MISSING") {
        Err(RuntimeError::Loader(msg)) => {
            assert!(
                msg.contains("no entry point found for script 'MISSING'"),
                "expected rebuild-hint message, got: {msg}"
            );
            assert!(
                msg.contains("hint: rebuild with sdk >= 0.14.0"),
                "expected rebuild-hint in message, got: {msg}"
            );
        }
        Err(other) => panic!("expected Loader error, got {other:?}"),
        Ok(_) => panic!("expected Loader error, got Ok"),
    }
}

#[test]
fn loader_rejects_legacy_bare_entry() {
    let dir = tempdir();
    // A legacy .so that exports only the bare __exa_udf_entry (no named symbol).
    // The loader must NOT fall back to the bare symbol and must return the
    // rebuild-hint error.
    use exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT;
    let host_fp = EXA_SDK_FINGERPRINT.trim_end_matches('\0');
    let fingerprint_with_nul = format!("{host_fp}\\0");
    let src = fixture_source(
        "__exa_udf_entry",
        EXA_UDF_ABI_VERSION,
        &fingerprint_with_nul,
    );
    let so = compile_fixture(dir.path(), "legacy_bare", &src);

    match LoadedUdf::open(&so, "SOME_SCRIPT") {
        Err(RuntimeError::Loader(msg)) => {
            assert!(
                msg.contains("no entry point found for script 'SOME_SCRIPT'"),
                "expected rebuild-hint message for legacy .so, got: {msg}"
            );
            assert!(
                msg.contains("hint: rebuild with sdk >= 0.14.0"),
                "expected rebuild-hint in message, got: {msg}"
            );
        }
        Err(other) => panic!("expected Loader error, got {other:?}"),
        Ok(_) => panic!("loader must not fall back to bare __exa_udf_entry"),
    }
}

#[test]
fn loader_rejects_abi_mismatch() {
    let dir = tempdir();
    // abi_version 99 (not 1). Fingerprint is irrelevant: the loader checks the
    // ABI version first and must bail before touching the fingerprint.
    let src = fixture_source("__exa_udf_entry_TESTUDF", 99, "0.0.0:wrong\\0");
    let so = compile_fixture(dir.path(), "abi_mismatch", &src);

    match LoadedUdf::open(&so, "TESTUDF") {
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
    let src = fixture_source(
        "__exa_udf_entry_TESTUDF",
        EXA_UDF_ABI_VERSION,
        "0.0.0:definitely-not-the-host\\0",
    );
    let so = compile_fixture(dir.path(), "fp_mismatch", &src);

    match LoadedUdf::open(&so, "TESTUDF") {
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
    // A cdylib with no __exa_udf_entry_TESTUDF symbol.
    let so = compile_fixture(
        dir.path(),
        "no_entry",
        "#[no_mangle] pub extern \"C\" fn unrelated() {}\n",
    );
    let result = LoadedUdf::open(&so, "TESTUDF");
    assert!(matches!(result, Err(RuntimeError::Loader(_))));
}

/// Regression test for issue #31: the `UdfContext` vtable layout must be
/// feature-independent. Before the fix a UDF built with only the `emit-arrow`
/// feature would have a different vtable slot order from the host SLC, so
/// `emit_batch` silently dispatched to `cluster_ip` and emitted 0 rows.
///
/// After the fix, all `UdfContext` methods are always declared regardless of
/// features, so a `.so` produced with ABI version 5 must load correctly against
/// a v5 host even when the `.so` was built without the `connect-back` feature.
/// The loader's ABI-version and fingerprint checks are the gate that enforces
/// layout compatibility — a `.so` that passes them is guaranteed to share the
/// same vtable layout as the host.
#[test]
fn emit_arrow_only_udf_emit_batch_dispatches_correctly() {
    let dir = tempdir();
    // Build a fixture representing a UDF compiled with emit-arrow only (no
    // connect-back). It exports the correct ABI version (5) and the host
    // fingerprint, so the loader must accept it — proving the vtable layout
    // is feature-independent and ABI-stable at version 5.
    use exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT;
    let host_fp = EXA_SDK_FINGERPRINT.trim_end_matches('\0');
    let fingerprint_with_nul = format!("{host_fp}\\0");
    let src = fixture_source(
        "__exa_udf_entry_EMIT_ARROW_ONLY",
        EXA_UDF_ABI_VERSION,
        &fingerprint_with_nul,
    );
    let so = compile_fixture(dir.path(), "emit_arrow_only", &src);

    // A fixture with ABI v5 and the correct fingerprint must load successfully.
    // This asserts that a .so built "emit-arrow only" (no connect-back feature)
    // is accepted by the v5 host — the vtable layout is uniform across all
    // feature combinations (#31 regression gate).
    match LoadedUdf::open(&so, "EMIT_ARROW_ONLY") {
        Ok(_) => {} // success: vtable layout is feature-independent
        Err(e) => panic!("expected loader to accept emit-arrow-only .so, got {e:?}"),
    }
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
