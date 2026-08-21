use super::*;

#[test]
fn committed_floor_is_the_published_container_floor() {
    assert_eq!(glibc_floor(), GlibcVersion::parse("2.41").unwrap());
}

#[test]
fn floor_check_rejects_newer_glibc() {
    let newer = GlibcVersion::parse("2.42").unwrap();
    assert_eq!(check_against_floor(&newer), FloorCompliance::ExceedsFloor);
}

#[test]
fn floor_check_accepts_glibc_at_the_floor() {
    assert_eq!(
        check_against_floor(&glibc_floor()),
        FloorCompliance::WithinFloor
    );
}

#[test]
fn floor_check_accepts_glibc_below_the_floor() {
    let older = GlibcVersion::parse("2.34").unwrap();
    assert_eq!(check_against_floor(&older), FloorCompliance::WithinFloor);
}

#[test]
fn committed_library_surface_is_parsed_and_names_libc() {
    assert!(!ALLOWED_SONAMES.is_empty());
    assert!(ALLOWED_SONAMES.contains(&"libc.so.6"));
}

#[test]
fn unknown_sonames_exclude_loader_and_vdso() {
    let needed = vec![
        "libc.so.6",
        "libssl.so.3",
        "ld-linux-x86-64.so.2",
        "ld-linux-aarch64.so.1",
        "linux-vdso.so.1",
    ];

    let unknown = unknown_sonames(needed);

    assert!(unknown.is_empty());
}

#[test]
fn unknown_sonames_reports_sonames_outside_the_allowlist() {
    let needed = vec!["libc.so.6", "libweirdthirdparty.so.1"];

    let unknown = unknown_sonames(needed);

    assert_eq!(unknown, vec!["libweirdthirdparty.so.1"]);
}
