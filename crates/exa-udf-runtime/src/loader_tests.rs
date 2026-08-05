use super::*;

// ---------------------------------------------------------------------------
// Helpers shared by inline loader tests
// ---------------------------------------------------------------------------

fn compile_vtable_fixture(
    out_dir: &std::path::Path,
    name: &str,
    abi_version: u32,
) -> std::path::PathBuf {
    let src = format!(
        r#"
use std::ffi::c_char;
use std::os::raw::c_void;

#[repr(C)]
pub struct ExaUdfVTable {{
    pub abi_version: u32,
    pub fingerprint: *const c_char,
    pub run: unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32,
    pub destroy: unsafe extern "C" fn(),
}}
unsafe impl Sync for ExaUdfVTable {{}}

unsafe extern "C" fn run_stub(_ctx: *mut c_void, _out: *mut *mut c_char) -> i32 {{ 0 }}
unsafe extern "C" fn destroy_stub() {{}}

static FP: &str = "0.0.0:stub\0";
static VT: ExaUdfVTable = ExaUdfVTable {{
    abi_version: {abi_version},
    fingerprint: FP.as_ptr() as *const c_char,
    run: run_stub,
    destroy: destroy_stub,
}};

#[no_mangle]
pub extern "C" fn __exa_udf_entry_TESTABI() -> *const ExaUdfVTable {{
    &VT as *const ExaUdfVTable
}}
"#
    );
    let src_path = out_dir.join(format!("{name}.rs"));
    let so_path = out_dir.join(format!("lib{name}.so"));
    std::fs::write(&src_path, &src).expect("write fixture source");
    let status = std::process::Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("-o")
        .arg(&so_path)
        .arg(&src_path)
        .status()
        .expect("invoke rustc");
    assert!(status.success(), "rustc failed for {name}");
    so_path
}

/// Compile a fixture whose vtable mirrors the host's full `ExaUdfVTable`
/// layout, including the trailing `output_shape` marker, so the loader may
/// soundly read that field. `output_shape` is the raw `OutputShape`
/// discriminant (0 = Returns, 1 = Emits). Uses the host fingerprint so the
/// `.so` passes the ABI/fingerprint gate.
fn compile_full_vtable_fixture(
    out_dir: &std::path::Path,
    name: &str,
    output_shape: u32,
) -> std::path::PathBuf {
    let abi = EXA_UDF_ABI_VERSION;
    let host_fp = EXA_SDK_FINGERPRINT.trim_end_matches('\0');
    let src = format!(
        r#"
use std::ffi::c_char;
use std::os::raw::c_void;

#[repr(u32)]
pub enum OutputShape {{ Returns = 0, Emits = 1 }}

#[repr(C)]
pub struct ExaUdfVTable {{
    pub abi_version: u32,
    pub fingerprint: *const c_char,
    pub run: unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32,
    pub destroy: unsafe extern "C" fn(),
    pub default_output_columns: Option<unsafe extern "C" fn(*mut *mut c_char) -> i32>,
    pub virtual_schema_adapter_call:
        Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_char) -> i32>,
    pub generate_sql_for_import_spec:
        Option<unsafe extern "C" fn(*const c_char, *mut *mut c_char) -> i32>,
    pub generate_sql_for_export_spec:
        Option<unsafe extern "C" fn(*const c_char, *mut *mut c_char) -> i32>,
    pub annotated_input_schema: *const c_char,
    pub annotated_output_schema: *const c_char,
    pub output_shape: OutputShape,
}}
unsafe impl Sync for ExaUdfVTable {{}}

unsafe extern "C" fn run_stub(_ctx: *mut c_void, _out: *mut *mut c_char) -> i32 {{ 0 }}
unsafe extern "C" fn destroy_stub() {{}}

static FP: &str = "{host_fp}\0";
static VT: ExaUdfVTable = ExaUdfVTable {{
    abi_version: {abi},
    fingerprint: FP.as_ptr() as *const c_char,
    run: run_stub,
    destroy: destroy_stub,
    default_output_columns: None,
    virtual_schema_adapter_call: None,
    generate_sql_for_import_spec: None,
    generate_sql_for_export_spec: None,
    annotated_input_schema: std::ptr::null(),
    annotated_output_schema: std::ptr::null(),
    output_shape: {output_shape_variant},
}};

#[no_mangle]
pub extern "C" fn __exa_udf_entry_SHAPE() -> *const ExaUdfVTable {{
    &VT as *const ExaUdfVTable
}}
"#,
        output_shape_variant = if output_shape == 0 {
            "OutputShape::Returns"
        } else {
            "OutputShape::Emits"
        },
    );
    let src_path = out_dir.join(format!("{name}.rs"));
    let so_path = out_dir.join(format!("lib{name}.so"));
    std::fs::write(&src_path, &src).expect("write fixture source");
    let status = std::process::Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .arg("-o")
        .arg(&so_path)
        .arg(&src_path)
        .status()
        .expect("invoke rustc");
    assert!(status.success(), "rustc failed for {name}");
    so_path
}

/// An EMITS-compiled `.so` validates clean against `Multiple` output but is
/// rejected against `ExactlyOnce` (RETURNS) with `OutputShapeMismatch`.
#[test]
fn output_shape_validated_against_output_iter() {
    let dir = make_tempdir();
    let so = compile_full_vtable_fixture(dir.path(), "shape_emits", 1);
    let udf = LoadedUdf::open(&so, "SHAPE").expect("full-vtable fixture must load");

    assert!(
        udf.validate_output_shape(IterType::Multiple).is_ok(),
        "EMITS marker must satisfy Multiple output"
    );
    match udf.validate_output_shape(IterType::ExactlyOnce) {
        Err(RuntimeError::OutputShapeMismatch {
            compiled,
            registered,
        }) => {
            assert_eq!(compiled, "EMITS");
            assert_eq!(registered, "RETURNS");
        }
        other => panic!("expected OutputShapeMismatch, got {other:?}"),
    }
}

fn make_tempdir() -> TempDir {
    let mut base = std::env::temp_dir();
    let unique = format!(
        "exa-loader-inline-{}-{}",
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
    path: std::path::PathBuf,
}
impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// ABI-version tripwire: a .so built against v4 must be rejected, not misdispatched
// ---------------------------------------------------------------------------

/// A `.so` built against ABI version 4 (the pre-#31-fix vtable layout) must
/// be rejected by the current loader with `AbiMismatch` — not loaded and
/// silently misdispatched, which was the failure mode #31 was designed to
/// prevent.
#[test]
fn current_abi_rejects_v4_so() {
    let dir = make_tempdir();
    let so = compile_vtable_fixture(dir.path(), "v4_fixture", 4);
    match LoadedUdf::open(&so, "TESTABI") {
        Err(RuntimeError::AbiMismatch { expected, found }) => {
            assert_eq!(
                expected, EXA_UDF_ABI_VERSION,
                "host must be the current ABI"
            );
            assert_eq!(found, 4, "fixture must present as ABI v4");
        }
        Err(other) => panic!("expected AbiMismatch, got {other:?}"),
        Ok(_) => panic!("loader must not accept a v4 .so against the current host"),
    }
}

/// Hook that writes a C-allocated error message into `*out` and returns 1.
unsafe extern "C" fn hook_error_with_msg(out: *mut *mut std::ffi::c_char) -> i32 {
    unsafe {
        *out = libc::strdup(c"hook returned this error".as_ptr());
    }
    1
}

/// Hook that leaves `*out` null and returns 1 (no message available).
unsafe extern "C" fn hook_error_null_out(out: *mut *mut std::ffi::c_char) -> i32 {
    let _ = out;
    1
}

/// Hook that writes a C-allocated result string into `*out` and returns 0.
unsafe extern "C" fn hook_success(out: *mut *mut std::ffi::c_char) -> i32 {
    unsafe {
        *out = libc::strdup(c"the value".as_ptr());
    }
    0
}

#[test]
fn error_text_surfaced_when_rc_nonzero() {
    let result = unsafe { call_noarg_hook("my_hook", hook_error_with_msg) };
    match result {
        Err(RuntimeError::Udf(msg)) => assert_eq!(msg, "hook returned this error"),
        other => panic!("expected Udf error, got {other:?}"),
    }
}

#[test]
fn generic_message_when_error_text_empty() {
    let result = unsafe { call_noarg_hook("my_hook", hook_error_null_out) };
    match result {
        Err(RuntimeError::Udf(msg)) => {
            assert!(
                msg.contains("returned error code"),
                "expected generic fallback message, got: {msg}"
            );
        }
        other => panic!("expected Udf error, got {other:?}"),
    }
}

#[test]
fn success_path_returns_written_string() {
    let result = unsafe { call_noarg_hook("my_hook", hook_success) };
    assert_eq!(result.unwrap(), "the value");
}
