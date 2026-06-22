# Tasks: remove-sdk-dead-public-api

## Phase 1: Implementation (Group A — concurrent, disjoint files)
- [x] 1.1 Remove `column_count`/`column_name`/`column_type`/`column_index`/`reset` from `UdfContext` (crates/exasol-udf-sdk/src/context.rs) — also dropped now-unused `ExaType` import
- [x] 1.2 Remove `impl TryFrom<f64> for Decimal` + its sub-assertions; rename test to `decimal_from_str_roundtrip` (crates/exasol-udf-sdk/src/value.rs)
- [x] 1.3 `usage()` returns `&'static str` (crates/exaudfclient/src/main.rs)
- [x] 1.4 Drop the `ctx.column_type(col)` sentence (docs/writing-a-udf.md)

## Phase 1: Implementation (Group B — after Group A)
- [x] 1.5 Bump version 0.14.0 → 0.15.0 (root Cargo.toml package + sdk pin) and regenerate Cargo.lock

## Phase 2: Verification
- [x] 2.1 Build (debug) — exit 0 (crates at v0.15.0)
- [x] 2.2 `cargo test --workspace --exclude it` — 0 failures
- [x] 2.3 `cargo clippy --all-targets --all-features -- -D warnings` — clean
- [x] 2.4 `cargo fmt --check` — no changes
- [x] 2.5 Scenario coverage audit + verification report
- [x] 2.6 Full E2E (scripts/ci-it-local.sh, live Docker) — green, rc=0 (gate re-run for the version bump)
