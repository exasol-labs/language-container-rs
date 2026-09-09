# Tasks: add-current-user-e2e-tests

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [x] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: SDK test-support surface)
- [x] 1.1 Add `test-support = []` feature to `crates/exasol-udf-sdk/Cargo.toml`; declare `#[cfg(any(test, feature = "test-support"))] pub mod test_support;` in `lib.rs`
- [x] 1.2 Write failing unit tests in `crates/exasol-udf-sdk/src/test_support_tests.rs`
- [x] 1.3 Implement `TestContext` in `crates/exasol-udf-sdk/src/test_support.rs` [expert]
- [x] 1.4 Implement `DefaultsCtx` plus its unit test
- [x] 1.5 Write rustdoc on `test_support`, `TestContext`, `DefaultsCtx`

## Phase 2: Implementation (Group B: Test-double migration)
- [x] 2.1 Add `exasol-udf-sdk` dev-dependency with `test-support` feature to the 15 consuming crates
- [x] 2.2 Replace 3 zero-column stubs with `DefaultsCtx`
- [x] 2.3 Replace 2 fixed-debug-level doubles
- [x] 2.4 Replace `RecordingCtx` in `output_shape.rs`
- [x] 2.5 Replace 8 scalar flat-row doubles
- [x] 2.6 Replace 4 SET cursor doubles
- [x] 2.7 Replace `handshake-meta` `MetaCtx` and `returns-with-emit` `TestCtx`
- [x] 2.8 Comment the 2 bespoke doubles kept in `connect_back.rs`/`feature_gate.rs`; confirm grep result

## Phase 2: Implementation (Group C: Identity fixture and live-DB scenarios)
- [x] 3.1 Commit `test-udfs/current-user-meta/` with unit test built on `TestContext`
- [x] 3.2 Commit workspace `Cargo.toml` members/default-members entries; add `-p current-user-meta` to CI allowlist
- [x] 3.3 Add `CURRENT_USER_META_LIB` const and `harness.upload_udf` call
- [x] 3.4 Add `current_user_meta_reports_session_user_and_open_schema` scenario
- [x] 3.5 Add `current_user_meta_current_schema_tracks_open_schema` and `current_user_meta_absent_current_schema` on side connection
- [x] 3.6 Add `current_user_meta_scope_user_is_view_owner` on side connection [expert]
- [x] 3.7 Add `current_user_meta_follows_impersonate`, last on side connection [expert]
- [x] 3.8 Wire the five scenario calls into `db_roundtrip_all_scenarios` in the fixed order

## Phase 2: Implementation (Group D: Release hygiene)
- [x] 4.1 Bump `[workspace.package].version` to `0.24.0`, update pinned `exasol-udf-sdk` dependency version, regenerate `Cargo.lock`

## Phase 4: Review Fixes
- [x] 4.2 [UNTESTED_ERROR_PATH] Add a `returns_ctx` helper (using `EmitPolicy::Reject`) to the `mod tests` of the 7 fixtures that lost RETURNS emit-ban coverage (`handshake-meta`, `json-parse`, `resolv-udf`, `scalar-double`, `timestamp-add-second`, `timestamp-passthrough`, `set-sum`); route their listed `TestContext::scalar(...)`/`TestContext::set(...)` call sites through it. Do not touch `returns-with-emit`, `emit-k`, `emit-bulk`, `set-filter`, `numeric-temporal-ingest`, `scalar-next-illegal`.
- [x] 4.3 [DEAD_FLEXIBILITY] Delete `ReturnPolicy` from `crates/exasol-udf-sdk/src/test_support.rs` (enum, `return_policy` field and initializer, `with_return_policy`), simplify `set_return` to unconditionally set `captured_return` and return `Ok(())`, and delete `return_policy_rejects_every_call_with_the_supplied_error` from `test_support_tests.rs`.
- [x] 4.4 [STANDARD_LIBRARY_DUPLICATE] Add `Clone` to `UdfError`'s derive list in `crates/exasol-udf-sdk/src/error.rs`; delete `replicate` from `test_support.rs` and replace its 3 call sites with `.clone()`.
- [x] 4.5 [CONTEXTLESS_ERROR] In `crates/it/tests/db_roundtrip.rs`, wrap the `EXA_DBA_SESSIONS` query's `USER_NAME`/`EFFECTIVE_USER` columns in `NVL(..., '<null>')` so a NULL collapses to the literal instead of SQL NULL, and reword the `ok_or_else` message to state no row exists for the session.
- [x] 4.6 [TACTICAL_SHORTCUT] In `current_user_meta_absent_current_schema` (`crates/it/tests/db_roundtrip.rs`), add an `eprintln!` prefixed `[it] current_user_meta_absent_current_schema:` reporting whether the field arrived as `<none>` or empty, following the style of the existing eprintln around lines 992-996; keep the assertion accepting both shapes.

## Phase 5: Verification
- [x] 5.1 Run build/test/lint/format checklist (integration suite deferred to implement-pr Phase B — needs SLC_TARBALL + Docker)
- [x] 5.2 Scenario coverage audit against plan's Scenario Coverage table (3 unit scenarios pass, 5 integration scenarios pending Phase B)
- [x] 5.3 Manual verification commands (4 of 6 run and pass; 2 deferred to Phase B)
