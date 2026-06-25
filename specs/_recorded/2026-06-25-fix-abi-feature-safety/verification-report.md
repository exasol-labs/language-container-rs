# Verification Report: fix-abi-feature-safety

**Generated:** 2026-06-25

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | #26 (`query_arrow` removed → arrow-free `ExaConnection`) and #31 (feature-independent `UdfContext` vtable + ABI v4→v5) implemented; all builds, lints, unit/integration tests, and the full live-DB E2E matrix (22 scenarios) green. |

| Check | Status |
|-------|--------|
| Build (default + all-features) | ✓ |
| Tests (cargo test) | ✓ |
| Integration / E2E (live DB) | ✓ |
| Lint (clippy -D warnings) | ✓ |
| Format (cargo fmt --check) | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ (covered by E2E roundtrip) |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed | Ignored |
|------|-----|--------|--------|---------|
| Unit + host integration (`cargo test`) | 163 | 159 | 0 | 4 |
| Live-DB E2E (`it` via `ci-it-local.sh`, 2026.1.0) | 22 scenarios | 22 | 0 | 0 |

E2E run with fix-validation memory config (`DB_MEM='4 GiB' MEM=12g SHM=2g`) to avoid the unrelated CI OOM repro; `Done (rc=0)`.

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
→ No issues found
```

### Formatter

```
cargo fmt --check
→ no diff
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| sdk | connect-back | ExaConnection arrow-free, always compiled | `crates/exasol-udf-sdk/tests/connect_back.rs` | `udfcontext_connect_back_methods_always_declared` + module compiles w/o feature | Pass |
| sdk | connect-back | query_arrow removed; mock implements query_for_each | `crates/exa-udf-runtime/tests/connect_back.rs` | `query_for_each_streams_value_rows` | Pass |
| sdk | connect-back | ConnectionObject public & unconditional | `crates/exasol-udf-sdk/src/abi.rs` (`#[cfg(test)]`) | `connect_back_types_compile_unconditionally` | Pass |
| sdk | udf-sdk | UdfContext vtable feature-independent (#31) | `crates/exa-udf-runtime/tests/loader.rs` | `emit_arrow_only_udf_emit_batch_dispatches_correctly` | Pass |
| sdk | udf-sdk | emit-arrow gates only dep + RecordBatch ext-trait | `crates/exasol-udf-sdk/tests/feature_gate.rs` | `emit_record_batch_ipc_present_without_emit_arrow` | Pass |
| sdk | udf-sdk | ABI bump rejects stale .so | `crates/exa-udf-runtime/src/loader.rs` (`#[cfg(test)]`) | `abi_version_5_rejects_v4_so` | Pass |
| runtime | connect-back-query | RuntimeExaConnection streams Value rows | `crates/exa-udf-runtime/tests/connect_back.rs` | `query_for_each_streams_value_rows` | Pass |
| runtime | (E2E) | connect-back read/stream/writeback in-DB | live-DB roundtrip | `connect_back_{query,stream,scalar,dml,writeback_same_schema,cluster_ip}` | Pass |

## Notes

- **Version**: workspace bumped `0.17.0 → 0.18.0` (breaking: `query_arrow` + `connect-back` feature removed, ABI v4→v5); `Cargo.lock` and the pinned `exasol-udf-sdk` workspace dep updated in sync. Stale debug `.so` test fixtures were rebuilt to carry the new fingerprint.
- **Code review** flagged two pre-existing, out-of-scope items left untouched (this plan targets #26/#31, not these):
  - `RuntimeExaConnection::query_for_each` uses `fetch_all()` rather than true one-batch-at-a-time wire streaming (justified by `current_thread` runtime re-entrancy deadlock; each batch is still dropped before its rows reach the callback). Strictly better than the removed `query_arrow`. Candidate follow-up.
  - `cb_log` writes to `/tmp/cb_debug.txt` on every connect-back call with no debug gate. Pre-existing; candidate follow-up issue.
- Two PR-introduced test-quality findings were fixed in-place: a meaningless ABI test made to actually name `ConnectionObject`/`ExaConnection`, and a duplicated `emit_record_batch_ipc_present_without_emit_arrow` test removed (canonical copy kept in `feature_gate.rs`).
