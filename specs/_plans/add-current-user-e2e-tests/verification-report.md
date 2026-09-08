# Verification Report: add-current-user-e2e-tests

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Build, unit tests, lint, and format are green. The five live-database scenarios need a built SLC tarball and a Docker Exasol instance, unavailable in this session; they run in Phase B of the implement-pr pipeline, which gates `/speq:record` on their result. |
| Code review | 5 findings — 5 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ (3 unit scenarios), pending (5 integration scenarios, deferred to Phase B) |
| Manual Tests | ✓ (4 of 6; 2 deferred to Phase B) |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | Not measured (no `cargo llvm-cov` run this session) |
| Integration | 0/5 new scenarios run this session; deferred to Phase B |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test`, default features) | All workspace crates except `it` | Green (exit 0) | — |
| Unit (`cargo test --all-features`) | All workspace crates except `it` | 344 | 2 |
| Integration (`cargo test -p it --features integration`) | Not run this session | — | — |

## Manual Tests

| Test | Result |
|------|--------|
| `cargo build --release -p current-user-meta` | ✓ exit 0, `target/release/libcurrent_user_meta.so` present |
| `cargo test -p exasol-udf-sdk --features test-support` | ✓ 0 failures |
| `cargo build -p exasol-udf-sdk` | ✓ exit 0, no `test_support` symbol in the produced library |
| `cargo test -p exasol-udf-sdk` (no feature) | ✓ 0 failures, `tests/feature_gate.rs` compiles without the feature |
| `cargo test -p it --features integration -- --nocapture` | Deferred to Phase B (needs `SLC_TARBALL` + Docker Exasol) |
| `cargo test -p it --features integration` after removing `-p current-user-meta` from CI | Deferred to Phase B |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.56s
```
0 warnings.

### Formatter

```
cargo fmt --check
```
Exit 0, no diff.

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| protocol | handshake | Identity metadata reports the executing user and the open schema | `crates/it/tests/db_roundtrip.rs` | `current_user_meta_reports_session_user_and_open_schema` | Pending (Phase B) |
| protocol | handshake | current_schema tracks the open schema independently of script_schema | `crates/it/tests/db_roundtrip.rs` | `current_user_meta_current_schema_tracks_open_schema` | Pending (Phase B) |
| protocol | handshake | A session with no open schema reports no current schema | `crates/it/tests/db_roundtrip.rs` | `current_user_meta_absent_current_schema` | Pending (Phase B) |
| protocol | handshake | scope_user reports the view owner when the script runs inside a view | `crates/it/tests/db_roundtrip.rs` | `current_user_meta_scope_user_is_view_owner` | Pending (Phase B) |
| protocol | handshake | IMPERSONATE establishes which user the current_user field reports | `crates/it/tests/db_roundtrip.rs` | `current_user_meta_follows_impersonate` | Pending (Phase B) |
| examples | test-udfs | current-user-meta reports the session identity fields as one string | `test-udfs/current-user-meta/src/lib_tests.rs` | `current_user_meta_joins_five_fields_and_marks_absent_optionals` | Pass |
| sdk | udf-sdk | The test-support feature ships a reusable UdfContext test double | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `test_context_covers_scalar_set_emit_and_return_paths` | Pass |
| sdk | udf-sdk | The test-support feature ships a defaults-preserving UdfContext double | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `defaults_ctx_overrides_no_provided_method` | Pass |

## Notes

Group C ran both `[expert]` scenarios (task 3.6, view owner; task 3.7, IMPERSONATE) live against a throwaway Exasol 2026.1 container during implementation, using a LUA stand-in script to validate the exact generated SQL. That probe is evidence the SQL and privilege sequences are correct, but it is not a substitute for the `crates/it/tests/db_roundtrip.rs` scenarios themselves running against the Rust SLC. Phase B builds the SLC tarball and runs `cargo test -p it --features integration` against the version matrix, which exercises the actual committed test code.

Code review found and fixed 5 standard findings: a dropped RETURNS emit-ban test path in 7 migrated fixtures, an unused `ReturnPolicy` knob in `TestContext` (deleted), a hand-rolled `Clone` for `UdfError` (replaced by deriving `Clone`), a NULL-column error message in the `EXA_DBA_SESSIONS` cross-check query (fixed with `NVL`), and a missing diagnostic in the absent-schema scenario (added). All 5 fixes are verified in the working tree; clippy and fmt stayed clean after the fix pass.

One residual risk carried from Group C, unresolved until Phase B: whether the impersonated user in `current_user_meta_follows_impersonate` can reach the SLC in BucketFS under the least-privilege grant set. The scenario's own code comment documents the stated remedy (widen to `GRANT DBA`) if that link fails.

The version bump (`0.23.0` → `0.24.0`) is out of scope for this report; it is release hygiene, not implementation behavior, and does not affect any test result above.
