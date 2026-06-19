# Tasks: fix-timestamp-timezone-handling

## Phase 2: Implementation (Group A — independent)
- [x] 2.1 Add `tzdata` to the `apk add --no-cache` line in the `Dockerfile.alpine` runtime stage
- [x] 2.2 Change `TIMESTAMP_EMIT` `%.6f`→`%.9f` in `crates/exa-udf-runtime/src/rowset.rs` + doc comment + nanosecond round-trip unit test (`timestamp_emit_nanosecond_roundtrip`)
- [x] 2.3 Scaffold `test-udfs/timestamp-add-second` crate (+`TestCtx` test `adds_one_second`, NULL→Null) and add to workspace `members`
- [x] 2.4 Scaffold `test-udfs/timestamp-now` crate (local wall-clock via zoneinfo-resolving crate; verify static-musl resolves named zone) and add to workspace `members` [expert]
- [x] 2.5 Scaffold `test-udfs/timestamp-passthrough` crate (+`TestCtx` test `passes_nanosecond_timestamp_through`) and add to workspace `members`

## Phase 2: Implementation (Group B — after A)
- [x] 2.6 Wire the 3 new UDF crates into the CI release artifact build (`.github/workflows/ci.yml`)
- [x] 2.7 Add `TS_ADD_LIB`/`TS_NOW_LIB`/`TS_PASS_LIB` consts + uploads + 3 scenarios (`timestamp_arithmetic_roundtrips`, `udf_local_time_matches_session_tz`, `timestamp_precision_matrix_roundtrips`) to `crates/it/tests/db_roundtrip.rs` [expert]

## Phase 3: Code Review
- [x] 3.1 Review all changed files — clean (one NIT fixed: Timestamp byte-cost estimate 26→29)

## Phase 4: Verification
- [x] 4.1 Build (`cargo build --release`) — PASS
- [x] 4.2 Test (`cargo test --workspace --exclude it`) — PASS (after rebuilding 0.12.1 fixtures)
- [x] 4.3 Lint (`cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings`) — PASS
- [~] 4.4 db-roundtrip suite — PARTIAL: precision proven (`timestamp_arithmetic_roundtrips` PASS); `udf_local_time_matches_session_tz` FAILED (UDF reports UTC, off by 7200s); `timestamp_precision_matrix_roundtrips` did not run. E2E gate NOT met → not recorded.

## Notes
- Task 12 (version bump 0.12.0→0.12.1) is handled by the orchestrator as an explicit step after implementation.
- Task 14 (PR) is handled by the orchestrator after `/speq:record`.
