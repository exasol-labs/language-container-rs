use std::path::{Path, PathBuf};
use std::process::Command;

const ABOVE_FLOOR_GLIBC_VERSION: &str = "2.99";
const ABOVE_FLOOR_PROBE_SYMBOL: &str = "exa_above_floor_probe";
const ABOVE_FLOOR_STUB_LINK_NAME: &str = "exalibcstub";
const UNSTAGED_PROBE_SYMBOL: &str = "exa_unstaged_probe";
const UNSTAGED_SONAME: &str = "libwidget.so.1";
const UNSTAGED_STUB_LINK_NAME: &str = "widget";
const PROBING_UDF_NAME: &str = "PROBE_IT";

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

/// The glibc floor the container publishes, read from the same committed file
/// the CLI reads, so an assertion here can never drift from the shipped value.
fn slc_glibc_floor() -> &'static str {
    include_str!("../slc-glibc-floor.txt").trim()
}

struct CdylibFixture<'a> {
    out_dir: &'a Path,
    name: &'a str,
    source: &'a str,
}

struct SharedStub<'a> {
    out_dir: &'a Path,
    file_name: &'a str,
    symbol: &'a str,
}

/// Compile `source` as a cdylib into `out_dir/lib<name>.so` and return the path.
fn compile_fixture(out_dir: &Path, name: &str, source: &str) -> PathBuf {
    compile_cdylib(
        CdylibFixture {
            out_dir,
            name,
            source,
        },
        &[],
    )
}

/// Compile `source` as a cdylib that links against `out_dir/lib<link_name>.so`,
/// recording an rpath so the fixture stays `dlopen`-able once validate reaches
/// its vtable probe.
fn compile_fixture_linked_against(fixture: CdylibFixture<'_>, link_name: &str) -> PathBuf {
    let out_dir = fixture.out_dir;
    compile_cdylib(
        fixture,
        &[
            format!("-Lnative={}", out_dir.display()),
            format!("-ldylib={link_name}"),
            format!("-Clink-arg=-Wl,-rpath,{}", out_dir.display()),
        ],
    )
}

fn compile_cdylib(fixture: CdylibFixture<'_>, extra_args: &[String]) -> PathBuf {
    let CdylibFixture {
        out_dir,
        name,
        source,
    } = fixture;
    let src_path = out_dir.join(format!("{name}.rs"));
    std::fs::write(&src_path, source).expect("write fixture source");
    let so_path = out_dir.join(format!("lib{name}.so"));

    let status = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2021")
        .args(extra_args)
        .arg("-o")
        .arg(&so_path)
        .arg(&src_path)
        .status()
        .expect("invoke rustc");
    assert!(status.success(), "rustc failed to compile fixture {name}");
    so_path
}

/// Compile a freestanding shared object exporting exactly `symbol`, used to give
/// a fixture a dynamic dependency the SLC does not provide.
fn compile_shared_stub(stub: SharedStub<'_>, link_args: &[String]) -> PathBuf {
    let SharedStub {
        out_dir,
        file_name,
        symbol,
    } = stub;
    let src_path = out_dir.join(format!("{symbol}.c"));
    std::fs::write(&src_path, format!("int {symbol}(void) {{ return 0; }}\n"))
        .expect("write stub source");
    let so_path = out_dir.join(file_name);

    let status = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("-nostdlib")
        .arg("-o")
        .arg(&so_path)
        .arg(&src_path)
        .args(link_args)
        .status()
        .expect("invoke cc");
    assert!(status.success(), "cc failed to compile stub {file_name}");
    so_path
}

/// Stage a stub that claims glibc's own soname and defines a symbol version
/// above the container's floor, so a fixture linking it records that version in
/// its `.gnu.version_r` table exactly as a too-new build host would.
fn compile_libc_stub_above_floor(out_dir: &Path) {
    let version_script = out_dir.join("above-floor.map");
    std::fs::write(
        &version_script,
        format!(
            "GLIBC_{ABOVE_FLOOR_GLIBC_VERSION} {{ global: {ABOVE_FLOOR_PROBE_SYMBOL}; local: *; }};\n"
        ),
    )
    .expect("write version script");
    compile_shared_stub(
        SharedStub {
            out_dir,
            file_name: &format!("lib{ABOVE_FLOOR_STUB_LINK_NAME}.so"),
            symbol: ABOVE_FLOOR_PROBE_SYMBOL,
        },
        &[
            "-Wl,-soname,libc.so.6".to_string(),
            format!("-Wl,--version-script={}", version_script.display()),
        ],
    );
}

/// Stage a stub under a soname outside the SLC library surface, present both
/// under its link name and under its soname so the fixture also loads.
fn compile_unstaged_stub(out_dir: &Path) {
    let so_path = compile_shared_stub(
        SharedStub {
            out_dir,
            file_name: &format!("lib{UNSTAGED_STUB_LINK_NAME}.so"),
            symbol: UNSTAGED_PROBE_SYMBOL,
        },
        &[format!("-Wl,-soname,{UNSTAGED_SONAME}")],
    );
    std::fs::copy(&so_path, out_dir.join(UNSTAGED_SONAME)).expect("stage stub under its soname");
}

/// The fingerprint the cargo-exasol-udf binary expects, taken from the linked
/// `exasol-udf-sdk` (the same constant the macro bakes into real `.so`s), with
/// the trailing C NUL stripped. A fixture baking this value validates as OK.
fn compute_expected_fingerprint() -> String {
    exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT
        .trim_end_matches('\0')
        .to_string()
}

/// The expected fingerprint as the fixture's vtable stores it — a C string, so
/// with the trailing NUL the loader reads back.
fn matching_fingerprint_with_nul() -> String {
    format!("{}\0", compute_expected_fingerprint())
}

fn vtable_source(abi_version: u32, fingerprint_with_nul: &str) -> String {
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
"#
    )
}

fn entry_point_source(udf_name: &str) -> String {
    format!(
        r#"
#[no_mangle]
pub extern "C" fn __exa_udf_entry_{udf_name}() -> *const ExaUdfVTable {{
    &VTABLE as *const ExaUdfVTable
}}
"#
    )
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
        "{}{}",
        vtable_source(abi_version, fingerprint_with_nul),
        entry_point_source(udf_name)
    )
}

/// Generate a cdylib source that exports TWO named entry points.
fn two_named_entries_fixture_source(abi_version: u32, fingerprint_with_nul: &str) -> String {
    format!(
        "{}{}{}",
        vtable_source(abi_version, fingerprint_with_nul),
        entry_point_source("DOUBLE_IT"),
        entry_point_source("TRIPLE_IT")
    )
}

/// Generate a cdylib source whose single named entry point calls `probe_symbol`,
/// so the linker must record the stub providing it as a real dynamic dependency.
fn probing_entry_fixture_source(
    abi_version: u32,
    fingerprint_with_nul: &str,
    probe_symbol: &str,
) -> String {
    format!(
        r#"{vtable}
unsafe extern "C" {{
    fn {probe_symbol}() -> u32;
}}

#[no_mangle]
pub extern "C" fn __exa_udf_entry_{PROBING_UDF_NAME}() -> *const ExaUdfVTable {{
    let _ = unsafe {{ {probe_symbol}() }};
    &VTABLE as *const ExaUdfVTable
}}
"#,
        vtable = vtable_source(abi_version, fingerprint_with_nul)
    )
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
fn validate_rejects_a_mistyped_deny_flag() {
    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", "lib.so", "--deny-unknown-dep"])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should fail for an unrecognized flag"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--deny-unknown-dep"),
        "error should name the unrecognized argument: {stderr}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// New tests for named-entry enumeration
// ─────────────────────────────────────────────────────────────────────────────

/// validate accepts a .so that exports one or more __exa_udf_entry_<NAME> symbols
/// with matching abi_version and sdk_fingerprint, and reports the artifact's
/// platform compatibility alongside the discovered names.
#[test]
fn validate_accepts_named_entries_and_reports_platform_summary() {
    let dir = tempfile::tempdir().expect("create tempdir");
    // abi_version must match what cargo-exasol-udf was compiled against.
    let src = two_named_entries_fixture_source(
        exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
        &matching_fingerprint_with_nul(),
    );
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
    assert!(
        stdout.contains(&format!("(SLC floor {})", slc_glibc_floor())),
        "output should report the glibc reference against the committed floor\nstdout={stdout}"
    );
    assert!(
        stdout.contains("dependencies: ") && stdout.contains("libc.so.6"),
        "output should report the artifact's dynamic dependencies\nstdout={stdout}"
    );
    assert!(
        stdout.contains("all within the SLC library surface"),
        "a plain cdylib links nothing outside the SLC surface\nstdout={stdout}"
    );
}

/// validate rejects a .so whose vtable has a wrong abi_version.
#[test]
fn validate_rejects_abi_mismatch() {
    let dir = tempfile::tempdir().expect("create tempdir");
    // Use abi_version 99 — intentionally wrong.
    let src = named_entry_fixture_source(99, &matching_fingerprint_with_nul(), "MY_UDF");
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
    let dir = tempfile::tempdir().expect("create tempdir");
    // Correct ABI version but wrong fingerprint — deliberately does not match the binary's RUNTIME_FINGERPRINT.
    let src = named_entry_fixture_source(
        exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
        "0.0.0:definitely-wrong-fingerprint\0",
        "MY_UDF",
    );
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
    let dir = tempfile::tempdir().expect("create tempdir");
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

#[test]
fn validate_rejects_non_elf_input() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let manifest = dir.path().join("Cargo.toml");
    std::fs::write(&manifest, "[package]\nname = \"not-an-elf\"\n").expect("write fixture");

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", manifest.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    assert!(
        !output.status.success(),
        "validate should fail for an input that is not a shared object"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a parseable ELF shared object"),
        "error should name the violated constraint: {stderr}"
    );
    assert!(
        stderr.contains("Cargo.toml"),
        "error should name the input: {stderr}"
    );
}

/// Verifies that the validate subcommand (and thus `enumerate_entry_symbols`)
/// correctly reports zero named entries for a `.so` that exports no
/// `__exa_udf_entry_*` symbols — exercising the predicate that `build::run`
/// relies on when checking the produced artifact.
#[test]
fn validate_rejects_a_so_without_any_named_entry_symbol() {
    let dir = tempfile::tempdir().expect("create tempdir");
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

// ─────────────────────────────────────────────────────────────────────────────
// Platform compatibility against the SLC container surface
// ─────────────────────────────────────────────────────────────────────────────

/// validate reports the artifact's own highest glibc reference — not merely the
/// floor it is compared against.
#[test]
fn validate_reports_glibc_floor_summary() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let src = named_entry_fixture_source(
        exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
        &matching_fingerprint_with_nul(),
        "SUMMARY_UDF",
    );
    let so = compile_fixture(dir.path(), "glibc_summary", &src);

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "a cdylib built on this host is below the SLC floor\nstdout={stdout}\nstderr={stderr}"
    );

    let floor = slc_glibc_floor();
    let summary = stdout
        .lines()
        .find(|line| line.starts_with("glibc: "))
        .unwrap_or_else(|| panic!("output should carry a glibc summary line\nstdout={stdout}"));
    let referenced = summary
        .strip_prefix("glibc: highest reference GLIBC_")
        .and_then(|rest| rest.split_once(" (SLC floor "))
        .map(|(version, _)| version)
        .unwrap_or_else(|| {
            panic!("summary should name the artifact's own highest reference: {summary}")
        });
    assert!(
        referenced.starts_with("2.")
            && referenced
                .split('.')
                .all(|component| component.parse::<u32>().is_ok()),
        "reported reference should be a numeric glibc 2.x version: {summary}"
    );
    assert!(
        summary.ends_with(&format!("(SLC floor {floor})")),
        "summary should name the committed floor {floor}: {summary}"
    );
}

/// validate fails an artifact referencing a glibc symbol version the container
/// does not ship — such an artifact can never load.
#[test]
fn validate_rejects_glibc_above_floor() {
    let dir = tempfile::tempdir().expect("create tempdir");
    compile_libc_stub_above_floor(dir.path());
    let src = probing_entry_fixture_source(
        exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
        &matching_fingerprint_with_nul(),
        ABOVE_FLOOR_PROBE_SYMBOL,
    );
    let so = compile_fixture_linked_against(
        CdylibFixture {
            out_dir: dir.path(),
            name: "above_floor",
            source: &src,
        },
        ABOVE_FLOOR_STUB_LINK_NAME,
    );

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "validate should fail an artifact above the SLC glibc floor\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains(&format!("GLIBC_{ABOVE_FLOOR_GLIBC_VERSION}")),
        "error should name the offending reference: {stderr}"
    );
    assert!(
        stderr.contains(&format!("floor {}", slc_glibc_floor())),
        "error should name the committed floor: {stderr}"
    );
    assert!(
        stderr.contains("libabove_floor.so"),
        "error should name the artifact: {stderr}"
    );
}

/// validate warns — and still succeeds — for an artifact whose dynamic
/// dependencies reach outside the staged SLC library surface.
#[test]
fn validate_warns_on_unknown_dt_needed() {
    let dir = tempfile::tempdir().expect("create tempdir");
    compile_unstaged_stub(dir.path());
    let src = probing_entry_fixture_source(
        exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
        &matching_fingerprint_with_nul(),
        UNSTAGED_PROBE_SYMBOL,
    );
    let so = compile_fixture_linked_against(
        CdylibFixture {
            out_dir: dir.path(),
            name: "unstaged_dep",
            source: &src,
        },
        UNSTAGED_STUB_LINK_NAME,
    );

    let output = Command::new(cargo_exasol_udf_bin())
        .args(["exasol-udf", "validate", so.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exasol-udf");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "an unstaged dependency warns by default, it does not fail\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("warning:") && stdout.contains("outside the SLC library surface"),
        "output should carry the unstaged-dependency warning\nstdout={stdout}"
    );
    assert!(
        stdout.contains(UNSTAGED_SONAME),
        "warning should name the unstaged soname\nstdout={stdout}"
    );
    assert!(
        stdout.contains(PROBING_UDF_NAME),
        "a warned artifact is still validated\nstdout={stdout}"
    );
}

/// validate escalates the same unstaged dependency to a failure when the caller
/// asks for the strict mode.
#[test]
fn validate_denies_unknown_dt_needed_with_flag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    compile_unstaged_stub(dir.path());
    let src = probing_entry_fixture_source(
        exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
        &matching_fingerprint_with_nul(),
        UNSTAGED_PROBE_SYMBOL,
    );
    let so = compile_fixture_linked_against(
        CdylibFixture {
            out_dir: dir.path(),
            name: "denied_dep",
            source: &src,
        },
        UNSTAGED_STUB_LINK_NAME,
    );

    let output = Command::new(cargo_exasol_udf_bin())
        .args([
            "exasol-udf",
            "validate",
            so.to_str().unwrap(),
            "--deny-unknown-deps",
        ])
        .output()
        .expect("failed to run cargo-exasol-udf");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "--deny-unknown-deps should fail an artifact with an unstaged dependency\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stderr.contains("outside the SLC library surface") && stderr.contains(UNSTAGED_SONAME),
        "error should name the unstaged soname: {stderr}"
    );
}

/// validate still succeeds under the strict flag when every dynamic dependency
/// is part of the staged SLC surface.
#[test]
fn validate_allows_staged_dt_needed_under_flag() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let src = two_named_entries_fixture_source(
        exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
        &matching_fingerprint_with_nul(),
    );
    let so = compile_fixture(dir.path(), "staged_deps_only", &src);

    let output = Command::new(cargo_exasol_udf_bin())
        .args([
            "exasol-udf",
            "validate",
            so.to_str().unwrap(),
            "--deny-unknown-deps",
        ])
        .output()
        .expect("failed to run cargo-exasol-udf");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "a cdylib linking only staged libraries passes the strict mode\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("all within the SLC library surface"),
        "output should report the dependency set as fully staged\nstdout={stdout}"
    );
    assert!(
        !stdout.contains("warning:"),
        "no warning is due for a fully staged dependency set\nstdout={stdout}"
    );
}
