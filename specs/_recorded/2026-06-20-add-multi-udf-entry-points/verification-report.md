# Verification Report: add-multi-udf-entry-points

## Verdict: PASS (unit + lint + format)

One `.so` now exports multiple `__exa_udf_entry_<NAME>` symbols; the loader resolves
the entry by the DB-sent `script_name`; the bare `__exa_udf_entry` is gone (hard break
with a rebuild-hint error). All unit tests, clippy (`-D warnings`), and `fmt --check` pass.
Live integration (`-p it --features integration`) and E2E (`make test-e2e`) are run by the
pipeline step that follows this report.

## Checklist

| Step | Command | Result |
|------|---------|--------|
| Unit tests | `cargo test` | PASS (rc 0, 0 failures) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | PASS (0 warnings) |
| Format | `cargo fmt --check` | PASS (clean) |
| Release build | `cargo build --release` | Deferred — UDF `.so` crates must build in the `rust:1.91` Docker (musl/glibc match); exercised by `make test-e2e`. Non-UDF crates compile-checked via clippy `--all-targets`. |

## Scenario Coverage

| Scenario | Test | Status |
|----------|------|--------|
| macro generates named entry point + vtable | `exasol-udf-macros` `entry_point_symbol_is_named` | PASS |
| fn name → UPPER_SNAKE SQL name | `fn_name_uppercased_to_sql_name` | PASS |
| `name =` overrides entry name | `name_attribute_overrides_entry_name` | PASS |
| invalid `name =` → clean compile error | trybuild `bad_name` | PASS |
| same-name annotations fail to link | trybuild `dup_entry` | PASS |
| distinct names → two entry points | `two_distinct_names_export_two_entries` | PASS |
| run shim catches panic | `run_shim_writes_malloc_backed_error_string_on_user_error` | PASS |
| loader accepts named entry | `loader_accepts_named_entry` | PASS |
| loader errors on missing named entry (rebuild hint) | `loader_errors_on_missing_named_entry` | PASS |
| legacy bare-entry `.so` rejected | `loader_rejects_legacy_bare_entry` | PASS |
| loader rejects ABI / fingerprint mismatch | `loader_rejects_abi_mismatch`, `loader_rejects_fingerprint_mismatch` | PASS |
| validate accepts named entries | `validate_accepts_named_entries` | PASS |
| validate rejects abi/fingerprint mismatch | `validate_rejects_abi_mismatch`, `validate_rejects_fingerprint_mismatch` | PASS |
| validate/build rejects `.so` with no named entry | `validate_rejects_no_named_entry_symbols`, `build_verifies_named_entry` | PASS |
| annotated-fixture exports two entries | fixture + dispatch tests | PASS |
| workspace version 0.14.0 | `Cargo.toml` / `Cargo.lock` | PASS |

## Notes

- Stale debug fixture `.so`s (`libscalar_double.so`, `libannotated_fixture.so`,
  `libsingle_call_fixture.so`) baked the old `0.13.1` fingerprint and were rebuilt
  (debug, host) after the version bump so `dispatch.rs`/`single_call.rs` pass.
- Pre-existing (out of scope): `crates/cargo-exasol-udf/build.rs:4` hardcodes
  `sdk_version = "0.1.1"` for its fingerprint, independent of the workspace version.
- Code review applied 7 fixes (invalid-name compile error, `nm`-absent message,
  multi-UDF sidecar warning, real build-verify test, dedup test, doc comments,
  consolidated `VTableProbe`).
