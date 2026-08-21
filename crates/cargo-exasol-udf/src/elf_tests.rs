use super::*;
use std::path::{Path, PathBuf};
use std::process::Command;

const ENTRY_FIXTURE_SOURCE: &str = r#"
extern "C" {
    fn __exa_udf_entry_GHOST() -> u32;
}

#[no_mangle]
pub extern "C" fn __exa_udf_entry_DOUBLE_IT() -> u32 { 2 }

#[no_mangle]
pub extern "C" fn __exa_udf_entry_TRIPLE_IT() -> u32 { 3 }

#[no_mangle]
pub extern "C" fn __exa_udf_entry() -> u32 { 1 }

#[no_mangle]
pub extern "C" fn unrelated_export() -> u32 { unsafe { __exa_udf_entry_GHOST() } }
"#;

fn write_source(dir: &Path, name: &str, source: &str) -> PathBuf {
    let src_path = dir.join(format!("{name}.rs"));
    std::fs::write(&src_path, source).expect("write fixture source");
    src_path
}

fn run_rustc(src_path: &Path, out_path: &Path, crate_args: &[&str]) {
    let status = Command::new("rustc")
        .args(crate_args)
        .arg("--edition=2021")
        .arg("-o")
        .arg(out_path)
        .arg(src_path)
        .status()
        .expect("invoke rustc");
    assert!(
        status.success(),
        "rustc failed to compile {}",
        src_path.display()
    );
}

fn compile_cdylib(dir: &Path, name: &str, source: &str) -> PathBuf {
    let out_path = dir.join(format!("lib{name}.so"));
    run_rustc(
        &write_source(dir, name, source),
        &out_path,
        &["--crate-type=cdylib"],
    );
    out_path
}

fn compile_object_file(dir: &Path, name: &str, source: &str) -> PathBuf {
    let out_path = dir.join(format!("{name}.o"));
    run_rustc(
        &write_source(dir, name, source),
        &out_path,
        &["--crate-type=lib", "--emit=obj"],
    );
    out_path
}

fn glibc(text: &str) -> GlibcVersion {
    GlibcVersion::parse(text).expect("fixture version parses")
}

#[test]
fn max_glibc_version_picks_highest_reference() {
    let highest = max_glibc_version(["GLIBC_2.2.5", "GLIBC_2.34", "GLIBC_2.9", "GLIBC_2.17"]);

    assert_eq!(highest, Some(glibc("2.34")));
}

#[test]
fn max_glibc_version_is_none_without_verneed() {
    let highest = max_glibc_version(std::iter::empty());

    assert_eq!(highest, None);
}

#[test]
fn max_glibc_version_ignores_non_numeric_glibc_versions() {
    let highest = max_glibc_version(["GLIBC_PRIVATE", "GLIBC_ABI_DT_RELR"]);

    assert_eq!(highest, None);
}

#[test]
fn max_glibc_version_ignores_other_version_namespaces() {
    let highest = max_glibc_version(["GCC_3.0", "OPENSSL_3.0.0", "ZLIB_1.2.9"]);

    assert_eq!(highest, None);
}

#[test]
fn glibc_version_orders_minor_components_numerically() {
    assert!(glibc("2.9") < glibc("2.34"));
    assert!(glibc("2.2.5") < glibc("2.3"));
    assert!(glibc("2.2") < glibc("2.2.5"));
    assert!(glibc("2.41") > glibc("2.40"));
}

#[test]
fn glibc_version_displays_the_text_it_parsed() {
    assert_eq!(glibc("2.41").to_string(), "2.41");
    assert_eq!(glibc("2.2.5").to_string(), "2.2.5");
}

#[test]
fn glibc_version_parse_rejects_malformed_text() {
    assert_eq!(GlibcVersion::parse(""), None);
    assert_eq!(GlibcVersion::parse("PRIVATE"), None);
    assert_eq!(GlibcVersion::parse("2."), None);
    assert_eq!(GlibcVersion::parse("2.x"), None);
    assert_eq!(GlibcVersion::parse("v2.41"), None);
}

#[test]
fn read_returns_only_the_defined_named_entry_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let so_path = compile_cdylib(dir.path(), "named_entries", ENTRY_FIXTURE_SOURCE);

    let mut udf_names = read(&so_path)
        .expect("fixture is a shared object")
        .udf_names;
    udf_names.sort();

    assert_eq!(udf_names, ["DOUBLE_IT", "TRIPLE_IT"]);
}

#[test]
fn read_returns_the_dt_needed_sonames() {
    let dir = tempfile::tempdir().unwrap();
    let so_path = compile_cdylib(dir.path(), "needed_sonames", ENTRY_FIXTURE_SOURCE);

    let needed = read(&so_path)
        .expect("fixture is a shared object")
        .needed_sonames;

    assert!(
        needed.iter().any(|soname| soname == "libc.so.6"),
        "a glibc cdylib must need libc.so.6, got {needed:?}"
    );
}

#[test]
fn read_returns_the_highest_referenced_glibc_version() {
    let dir = tempfile::tempdir().unwrap();
    let so_path = compile_cdylib(dir.path(), "glibc_reference", ENTRY_FIXTURE_SOURCE);

    let referenced = read(&so_path)
        .expect("fixture is a shared object")
        .max_glibc_version
        .expect("a glibc cdylib references versioned glibc symbols");

    assert!(
        referenced >= glibc("2.2.5") && referenced < glibc("3.0"),
        "expected a glibc 2.x reference, not a version from another namespace: {referenced}"
    );
}

#[test]
fn read_reports_a_non_elf_file_as_not_a_shared_object() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Cargo.toml");
    std::fs::write(&path, "[package]\nname = \"not-an-elf\"\n").unwrap();

    let error = read(&path).expect_err("a TOML file is not a shared object");

    assert!(matches!(error, ElfError::NotASharedObject { .. }));
    let message = error.to_string();
    assert!(
        message.contains("not a parseable ELF shared object"),
        "message should name the constraint: {message}"
    );
    assert!(
        message.contains("Cargo.toml"),
        "message should name the input: {message}"
    );
}

#[test]
fn read_reports_an_elf_object_file_as_not_a_shared_object() {
    let dir = tempfile::tempdir().unwrap();
    let object_path =
        compile_object_file(dir.path(), "relocatable", "pub fn answer() -> u32 { 42 }");

    let error = read(&object_path).expect_err("a relocatable object is not a shared object");

    assert!(matches!(error, ElfError::NotASharedObject { .. }));
    let message = error.to_string();
    assert!(
        message.contains("relocatable.o"),
        "message should name the input: {message}"
    );
}

#[test]
fn read_reports_an_unreadable_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("absent.so");

    let error = read(&missing).expect_err("a missing path cannot be read");

    assert!(matches!(error, ElfError::Unreadable { .. }));
    assert!(
        error.to_string().contains("absent.so"),
        "message should name the input: {error}"
    );
}
