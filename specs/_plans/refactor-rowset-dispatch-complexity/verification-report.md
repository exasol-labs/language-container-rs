# Verification Report: refactor-rowset-dispatch-complexity

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Cognitive-complexity and duplication refactor of `rowset.rs`/`dispatch.rs`/`single_call.rs` complete; emit-buffer invariants pinned and preserved; coverage residuals closed to their unit-reachable floor; CI coverage job hardened; workspace version bumped 0.21.1 → 0.21.2; all checks green including a live Docker-backed integration replay run twice (before and after review fixes). |
| Code review | 8 findings — standard: 6, expert: 2 — all 8 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

Measured via `cargo llvm-cov -p exa-udf-runtime -p exaudfclient --show-missing-lines` after all fixes:

| File (plan's Phase 5 targets) | Line Coverage | Missed Lines | Residual category |
|---|---|---|---|
| `rowset.rs` | 98.18% | 18 | Unreachable-in-practice arms (Arrow-invariant-guaranteed downcasts; `getifaddrs` syscall-failure/interface-down branches; a provably-dead post-`push_batch` flush check) — enumerated by task 5.2's agent |
| `dispatch.rs` | 98.11% | 3 | One residual: `emit_flusher`'s zero-row no-op guard, unreachable from any real wire scenario (only invoked when `should_flush()` is already true) |
| `single_call.rs` | 100.00% | 0 | Fully closed |

| Type | Coverage % |
|------|------------|
| Unit (3 target files, aggregate) | ~98% (see per-file table above) |
| Integration | 34/34 scenarios pass (see Scenario Coverage) |

Out-of-scope-by-plan, not gated: `connect_back.rs` (24.19%, DB-session-establishment lines explicitly excluded by the plan's Non-Goals) and the new `wire.rs` (88.64%, not one of the plan's three named target files — its ping-retry loop path is now covered by an extended two-consecutive-ping test, added as an expert review fix).

### Test Results

| Type | Run | Passed | Failed | Ignored |
|------|-----|--------|--------|---------|
| Unit (`cargo test`, default-members) | 1 | 300 | 0 | 2 |
| Unit (`cargo test -p exa-udf-runtime --all-features`) | 2 (before + after review fixes) | 164 each run | 0 | 0 |
| Integration (`cargo test -p it --features integration`, live Docker DB) | 2 (before + after review fixes) | 1 (34 scenarios internally) | 0 | 0 |
| Lint (`cargo clippy --all-targets --all-features -- -D warnings`) | 2 | — | 0 warnings/errors | — |
| Format (`cargo fmt --check`) | 2 | — | 0 diffs | — |

### Manual Tests

| Test | Result |
|------|--------|
| `cargo test -p exa-udf-runtime --all-features` (fixtures: scalar-double, annotated-fixture, single-call-fixture) | ✓ |
| `cargo test -p it --features integration` (local Exasol Docker DB, `scripts/ci-it-local.sh DB_MEM='4 GiB' MEM=12g SHM=2g`) | ✓ — all 34 scenarios ok, `rc=0` |
| `cargo test -p exa-udf-runtime --test single_call` | ✓ — 17 tests (plan expected 5+) |
| `cargo llvm-cov -p exa-udf-runtime -p exaudfclient --show-missing-lines` | ✓ — no missing lines in `rowset.rs`/`dispatch.rs`/`single_call.rs` except the documented not-unit-reachable residuals above |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.60s
exit 0, zero warnings
```

### Formatter

```
cargo fmt --check
exit 0, no diff
```

## Scenario Coverage

All scenarios are the existing contracts of the four bounding features (`runtime/rowset-codec`, `runtime/emit-arrow-batch`, `runtime/dispatch-run-loop`, `runtime/dispatch-single-call`) — UNCHANGED per this plan's Features section. Full scenario-to-test mapping is in plan.md's Verification > Scenario Coverage table; key NEW pins added by this plan:

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| runtime | rowset-codec | EmitBuffer tracks a running byte estimate and reports when to flush | `crates/exa-udf-runtime/src/rowset_tests.rs` | `emit_buffer_limit_is_exactly_4_000_000` | Pass |
| runtime | rowset-codec | Row-path emit flushes exactly once mid-run, buffers residual for tail flush | `crates/exa-udf-runtime/src/rowset_tests.rs` | `bridge_emit_row_path_flushes_once_mid_run_and_buffers_residual` | Pass |
| runtime | dispatch-run-loop | UDF error closes the session with a prefixed message | `crates/exa-udf-runtime/tests/dispatch.rs` | `udf_error_closes_session_with_prefixed_message` | Pass |
| runtime | dispatch-run-loop | Dispatch reads UDF error text from the run out-pointer | `crates/exa-udf-runtime/tests/dispatch.rs` | `run_error_out_pointer_text_reaches_close` | Pass |
| runtime | dispatch-run-loop | Mid-group MT_CLEANUP ends session cleanly | `crates/exa-udf-runtime/tests/dispatch.rs` | `mid_group_cleanup_ends_session_cleanly` | Pass |
| runtime | dispatch-run-loop | Mid-group MT_CLOSE surfaces message | `crates/exa-udf-runtime/tests/dispatch.rs` | `mid_group_close_surfaces_message` | Pass |
| runtime | dispatch-run-loop | Ping-pong mid-exchange retries transparently (extended to 2 consecutive pings) | `crates/exa-udf-runtime/tests/dispatch.rs` | `ping_pong_mid_exchange_retries_transparently` | Pass |
| runtime | dispatch-single-call | Unexpected event in single-call mode is a hard error | `crates/exa-udf-runtime/tests/single_call.rs` | `unexpected_event_in_single_call_mode_is_hard_error` | Pass |
| runtime | rowset-codec | Arrow-cell→Value conversion is byte-identical across the row and batch encoder paths | `crates/exa-udf-runtime/src/rowset_tests.rs` | `encode_slice_matches_row_path_across_every_block_type` | Pass |
| runtime | rowset-codec | Negative-nanosecond timestamps yield the correct pre-epoch instant (not the Unix epoch) | `crates/exa-udf-runtime/src/rowset_tests.rs` | `accessor_value_timestamp_nanosecond_negative_yields_pre_epoch_instant` | Pass |

Integration (live-DB) scenario coverage — all 34 scenarios in `crates/it/tests/db_roundtrip.rs`'s `db_roundtrip_all_scenarios` passed on both the pre-review-fix and post-review-fix runs, including `emit_bulk_boundary_rows_and_oversize_row` (the oversized-single-row emit invariant) and all `connect_back_*` scenarios.

## Notes

- **Phase ordering honored**: Phase 1 (emit invariant pins) landed and was verified green before any Phase 3/4 restructuring, per the plan's normative ordering.
- **Code review found one real behavior-preserving-refactor risk and fixed it**: `accessor_value`'s `TsNanosecond` arm used truncating `/`/`%` on `ns`, which silently degraded every pre-epoch nanosecond-unit Arrow timestamp to the Unix epoch (silent data corruption). Fixed to `div_euclid`/`rem_euclid`; the four cross-path byte-identity guards stayed green, confirming the fix doesn't break row/batch parity.
- **Second review finding of note**: `wire::request`'s ping-retry was unbounded recursion (one stack frame per consecutive DB ping on the crate's single shared wire path used by both dispatchers). Converted to a loop, behavior-identical, and the ping-pong test was extended to cover two consecutive pings, not just one.
- **Not unit-reachable, explicitly accepted** (per plan's requirement to document DB-bound and equivalent gaps): `connect_back.rs`'s `open_connect_back` exarrow session establishment (needs a live DB, out of this plan's scope per Non-Goals); a few `rowset.rs` arms guaranteed unreachable by Arrow's own type invariants (e.g. a `Utf8`-typed column always downcasting to `StringArray`) or by real OS/network state (`getifaddrs` syscall failure); `dispatch.rs`'s `emit_flusher` zero-row guard, proven dead by the cost-formula argument in task 5.2/5.3's reports; `/proc/self/statm` read-failure fallback (needs a filesystem-failure mock).
- **Sonar acceptance is not locally verifiable** (per plan's Verification section) — confirmed only after this PR's CI run on SonarCloud. Local proxy used throughout: every restructured function composes sequentially with no nested control flow beyond one level (`run_group`: complexity 32 → 3; `compute_row_costs`/`accessor_value`/`impl UdfContext` bodies: one owning function or macro each, duplication removed at the source).
- **CI coverage job hardened** (task 2.1): fixture cdylib build list extended from 3 to all 8 `dlopen`-target fixtures (cold-cache-safe); `--all-features` added to the `cargo llvm-cov` invocation to make the measured feature set explicit rather than inherited.
- **Version bump**: `[workspace.package].version` and the pinned `exasol-udf-sdk` dependency entry both 0.21.1 → 0.21.2; `Cargo.lock` regenerated (not hand-edited); all local test-udf `.so` fixtures (release and debug profiles) rebuilt to match the new ABI fingerprint.
- **Verification checklist was run twice in full** (build, test, all-features test, clippy, fmt, live Docker integration) — once before code review, once after all 8 review findings were fixed — both fully green, including two independent live-DB integration replays.
