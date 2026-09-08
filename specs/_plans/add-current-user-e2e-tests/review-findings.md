# Code Review Findings: add-current-user-e2e-tests

## Summary
- Files reviewed: 47
- Total findings: 5 (standard: 5, expert: 0)

Evidence gathered before writing: `cargo clippy --all-targets --all-features -- -D warnings` exits 0, and `cargo clippy -p it --all-targets --features integration -- -D warnings` exits 0, so no unused import, unused variable, or suppressed warning survives the migration. `grep -rn "UdfContext for"` reports exactly `rowset.rs` (2 host bridges), `test_support.rs` (2 doubles), `connect_back.rs`, and `feature_gate.rs`, so task 2.8's migration target is met and no orphaned double remains. Both kept bespoke doubles carry the required rationale comment. The IMPERSONATE scenario is coded as intended: it asserts that `current_user` equals exactly one of `EXA_DBA_SESSIONS.USER_NAME` or `EFFECTIVE_USER`, records the matching column through `eprintln!`, and hardcodes neither.

## Standard fixes

### test-udfs/{handshake-meta,json-parse,resolv-udf,scalar-double,timestamp-add-second,timestamp-passthrough,set-sum}/src/lib.rs

#### [UNTESTED_ERROR_PATH] The migration dropped the RETURNS emit ban from 7 fixtures
- Location: `handshake-meta` line 29; `json-parse` lines 27, 34, 41, 48, 55; `resolv-udf` lines 29, 38, 47; `scalar-double` lines 41, 48, 55, 62; `timestamp-add-second` lines 40, 47; `timestamp-passthrough` lines 36, 43; `set-sum` lines 28, 38, 44
- Issue: each of these 7 deleted doubles rejected `emit` — six with `UdfError::Unimplemented("emit is banned in RETURNS output")` and `set-sum` with `UdfError::User("set-sum must not emit")` (verified against `git show HEAD:<file>`). The replacement `TestContext::scalar(...)` / `TestContext::set(...)` uses the default `EmitPolicy::Record`, so a RETURNS-shaped fixture that started calling `ctx.emit` would now record the row and keep passing. The `sdk/udf-sdk` spec delta states the emit policy exists "so a fixture can assert the runtime's ban on `emit` in RETURNS output", yet `EmitPolicy::Reject` has no consumer anywhere in the repository — the capability was built and then not applied at the exact 20 sites it was built for.
- Fix: In each of the 7 listed `src/lib.rs` files, add a private helper to the `mod tests` module that wraps the constructor with the ban that file's deleted double enforced — for the six `Unimplemented` fixtures: `fn returns_ctx(row: Vec<Value>) -> TestContext { TestContext::scalar(row).with_emit_policy(EmitPolicy::Reject(UdfError::Unimplemented("emit is banned in RETURNS output".into()))) }`, and for `set-sum`: `fn returns_ctx(rows: Vec<Vec<Value>>) -> TestContext { TestContext::set(rows).with_emit_policy(EmitPolicy::Reject(UdfError::User("set-sum must not emit".into()))) }`. Import `EmitPolicy` alongside `TestContext`, and route every `TestContext::scalar(...)` / `TestContext::set(...)` at the listed lines through that helper. Do not touch `returns-with-emit`, `emit-k`, `emit-bulk`, `set-filter`, `numeric-temporal-ingest`, or `scalar-next-illegal`: their deleted doubles accepted `emit` (or rejected `next`) and their current form is correct.

### crates/exasol-udf-sdk/src/test_support.rs

#### [DEAD_FLEXIBILITY] ReturnPolicy is a knob no caller varies and no spec requires
- Location: lines 47-56 (`ReturnPolicy`), line 85 (`return_policy` field), line 114 (initializer), lines 144-148 (`with_return_policy`), lines 283-291 (`set_return` match)
- Issue: `with_return_policy` has no consumer outside `test_support_tests.rs`, no surveyed double rejected `set_return` (the only test of that failure path, `context_tests.rs::default_set_return_unimplemented`, uses `DefaultsCtx` and the trait default), and the `sdk/udf-sdk` spec delta requires a caller-supplied error for `emit` and `next` only. The plan's own decision table rejects a knob for the out-of-range `get` error on the ground that "a knob here would leak a test detail into the interface"; the same reasoning applies here, and this one also enlarges a public API surface the SDK must keep.
- Fix: In crates/exasol-udf-sdk/src/test_support.rs, delete the `ReturnPolicy` enum, the `return_policy` struct field and its initializer in `positioned`, and the `with_return_policy` method; reduce `set_return` to unconditionally set `self.captured_return = Some(value)` and return `Ok(())`. Delete the `return_policy_rejects_every_call_with_the_supplied_error` test from crates/exasol-udf-sdk/src/test_support_tests.rs. Keep `EmitPolicy` and `NextPolicy` unchanged, and keep `captured_return_separates_never_called_from_called_with_none`.

#### [STANDARD_LIBRARY_DUPLICATE] `replicate` hand-rolls Clone for UdfError
- Location: lines 385-392, with call sites at lines 267, 273, and 289
- Issue: `replicate` matches all four `UdfError` variants and rebuilds each from a cloned `String` — that is exactly `#[derive(Clone)]`. Every variant added to `UdfError` (crates/exasol-udf-sdk/src/error.rs, an enum of four `String`-only variants) costs an edit here, reintroducing the per-site maintenance cost this whole plan set out to remove.
- Fix: Add `Clone` to the derive list on `UdfError` in crates/exasol-udf-sdk/src/error.rs so it reads `#[derive(Debug, Clone, Error)]`. In crates/exasol-udf-sdk/src/test_support.rs, delete the `replicate` function and replace each of its three call sites with `error.clone()`.

### crates/it/tests/db_roundtrip.rs

#### [CONTEXTLESS_ERROR] A NULL EFFECTIVE_USER is reported as a missing session row
- Location: lines 962-977
- Issue: the query concatenates `USER_NAME || '|' || EFFECTIVE_USER`, so a NULL in either column collapses the whole cell to SQL NULL and `query_single_string` returns `None`. The row for `CURRENT_SESSION` always exists, so the `None` branch can fire for no other reason — yet its message claims "EXA_DBA_SESSIONS reported no USER_NAME/EFFECTIVE_USER pair for session {session_id}", pointing a maintainer at a missing row instead of at the NULL column. That failure is reachable on the older entries of the CI version matrix, where the population of `EFFECTIVE_USER` under IMPERSONATE is unverified (the plan's probe covered 2026.1 only).
- Fix: In crates/it/tests/db_roundtrip.rs, change the `EXA_DBA_SESSIONS` query at line 965 to `SELECT CAST(NVL(USER_NAME, '<null>') || '|' || NVL(EFFECTIVE_USER, '<null>') AS VARCHAR(256))` so a NULL column reaches the comparison as the literal `<null>` and is reported by the existing "must equal exactly one of" bail arm, which already prints both values. Reword the `ok_or_else` message at lines 970-974 to state that no row exists for the session.

#### [TACTICAL_SHORTCUT] The absent-schema scenario records nothing about which shape it observed
- Location: lines 780-793
- Issue: `current_user_meta_absent_current_schema` is documented as "the only end-to-end coverage of the `optional` handshake field arriving absent", but the assertion passes both when the field is omitted (rendered as `<none>`) and when it arrives present-but-empty, so a green run is no evidence that the absent path was exercised. The planning notes record the choice as deliberate ("whether the DB omits the field or sends it empty is unverified"), yet nothing in the run reports which of the two the database produced, so the shortcut has no path to being tightened later. The sibling IMPERSONATE scenario already sets the precedent of printing the discovered value where the mapping is undocumented.
- Fix: In crates/it/tests/db_roundtrip.rs, in `current_user_meta_absent_current_schema`, after the check at lines 787-793 add an `eprintln!` in the style of the one at lines 992-996 that reports whether the field arrived as the `<none>` marker or as an empty string, prefixed `[it] current_user_meta_absent_current_schema:`. Leave the assertion accepting both shapes.

## Expert fixes
[none]
