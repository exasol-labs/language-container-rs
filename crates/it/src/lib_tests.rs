use super::{db_series, db_tag, decode_base64};
use std::sync::Mutex;

// Serialise tests that mutate process-wide env vars. The Rust test harness
// runs tests in parallel by default, so concurrent env-var writes cause
// spurious failures without this guard.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn decodes_known_base64() {
    assert_eq!(decode_base64("aGVsbG8=").unwrap(), "hello");
    assert_eq!(decode_base64("aGpYMHM4dE5zSk1n").unwrap(), "hjX0s8tNsJMg");
}

#[test]
fn db_series_returns_env_var_when_recognised() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var("EXASOL_DB_SERIES", "2025-1") };
    let result = db_series();
    unsafe { std::env::remove_var("EXASOL_DB_SERIES") };
    assert_eq!(result, "2025-1");
}

#[test]
fn db_series_fallback_matches_enabled_feature() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("EXASOL_DB_SERIES") };
    let series = db_series();
    assert!(
        ["2025-1", "2025-2", "2026-1"].contains(&series.as_str()),
        "db_series() fallback returned unexpected value: {series:?}"
    );
}

#[test]
fn db_tag_uses_exasol_version_override() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("EXASOL_DB_SERIES") };
    unsafe { std::env::set_var("EXASOL_VERSION", "2025.1.99") };
    let tag = db_tag();
    unsafe { std::env::remove_var("EXASOL_VERSION") };
    assert_eq!(tag, "2025.1.99");
}

#[test]
fn db_tag_maps_series_to_known_image_tags() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::remove_var("EXASOL_VERSION") };
    let series_to_tag = [
        ("2025-1", "2025.1.11"),
        ("2025-2", "2025.2.1"),
        ("2026-1", "2026.1.0"),
    ];
    for (series, expected_tag) in series_to_tag {
        unsafe { std::env::set_var("EXASOL_DB_SERIES", series) };
        let tag = db_tag();
        unsafe { std::env::remove_var("EXASOL_DB_SERIES") };
        assert_eq!(
            tag, expected_tag,
            "series {series:?} should map to {expected_tag:?}"
        );
    }
}
