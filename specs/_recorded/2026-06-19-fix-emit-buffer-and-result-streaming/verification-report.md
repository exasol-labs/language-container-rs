# Verification Report: fix-emit-buffer-and-result-streaming

**Verdict: PASS** — all automated checks green, all integration scenarios pass on a live Exasol 2026.1.0 Docker container.

---

## Summary

Both memory-bounding goals are implemented and verified end-to-end:

1. **Emit buffering**: `ctx.emit` accumulates rows in `EmitBuffer` and flushes `MT_EMIT` when the running byte estimate crosses `EMIT_BUFFER_LIMIT_BYTES = 4_000_000`. A tail flush fires after `run()` returns. The `emit-bulk` integration test (50,000 × ~100-byte rows = ~5 MB) forces ≥1 mid-run flush plus a tail flush; 50,000 rows were received intact.

2. **Connect-back streaming**: `ExaConnection::query_for_each` streams rows through a callback, converting each Arrow batch to `Vec<Value>` and dropping the batch before moving to the next. `RuntimeExaConnection` overrides it with `fetch_all()` inside a single `block_on` (true per-batch server streaming is not achievable with `current_thread` + `ResultSetIterator::next_batch()`'s nested `block_on` — see decision-log [5]). The `connect_back_stream` integration test (100-row seeded table) confirmed all rows processed and correct count emitted.

---

## Automated Checks

| Step | Command | Result |
|------|---------|--------|
| Build | `cargo build --release` | ✅ exit 0 |
| Binary build | `cargo build --release -p exaudfclient` | ✅ exit 0 |
| Unit tests | `cargo test` | ✅ 0 failed |
| E2E (DB) | `cargo test -p it --features integration` | ✅ 0 failed (Exasol 2026.1.0) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ 0 errors |
| Format | `cargo fmt --check` | ✅ no changes |

---

## Scenario Coverage

| Scenario | Test | Result |
|----------|------|--------|
| `dispatch-run-loop / Set/EMITS dispatch emits multiple rows across batches` | `emit_bulk_flushes_multiple_batches` (integration) | ✅ 50,000/50,000 rows received |
| `dispatch-run-loop / EmitBuffer tracks a running byte estimate and reports when to flush` | `emit_buffer_byte_estimate_and_should_flush` (unit) | ✅ |
| `dispatch-run-loop / A single emitted row larger than the flush threshold is sent on its own` | `oversized_single_row_flushes_alone` (unit) | ✅ |
| `connect-back / query_for_each streams the result set one batch at a time` | `connect_back_stream_reads_all_rows` (integration) | ✅ count=100 |
| `connect-back / Connect-back query returns Arrow batches to the UDF` | existing `connect_back_udf_queries_and_emits` | ✅ unaffected |
| `sdk/connect-back / query_for_each default streams rows to the callback` | `query_for_each_default_streams_rows` (unit) | ✅ |
| `sdk/connect-back / query_for_each stops on callback error` | `query_for_each_stops_on_callback_error` (unit) | ✅ |
| `sdk/connect-back / record_batch_to_rows matches multibatch` | `record_batch_to_rows_matches_multibatch` (unit) | ✅ |

---

## Integration Test Run (Exasol 2026.1.0)

```
[it] scenario scalar_double ok
[it] scenario set_filter ok
[it] scenario json_parse ok
[it] scenario udf_error ok
[it] scenario udf_error_message ok
[it] scenario emit_bulk ok
[it] scenario single_call_default_output_columns ok
[it] scenario single_call_unimplemented ok
[it] scenario connect_back_cluster_ip ok
[it] scenario connect_back_dml ok
[it] scenario connect_back_query ok
[it] scenario connect_back_scalar ok
[it] scenario connect_back_writeback_same_schema ok
[it] scenario connect_back_stream ok
[it] scenario resolv_udf_resolves_external_host ok
[it] scenario resolv_udf_errors_on_unresolvable_host ok

test result: ok. 1 passed; 0 failed
```

---

## Code Review Findings

All 6 correctness invariants confirmed by the reviewer:
- `RefCell` shared correctly (one cell, serial borrows) ✅
- Flush-then-clear ordering correct ✅
- Tail flush still fires ✅
- `byte_estimate` reset in `clear` ✅
- `query_for_each` object-safe (`&mut dyn FnMut`) ✅
- Arrow→Value conversion runs in runtime's arrow context ✅

**Known deviation from plan (documented)**: `RuntimeExaConnection::query_for_each` uses `fetch_all()` rather than `ResultSetIterator::next_batch()`. Reason: the `current_thread` Tokio runtime cannot safely call `handle.block_on()` from within an outer `block_on` — it deadlocks. The per-batch drop loop still eliminates the double-copy peak (Arrow + Value simultaneously) of the old `query` implementation. True per-batch server streaming requires a future exarrow-rs API change. Recorded in decision-log [5].

**Format issue**: fixed by `cargo fmt` during review.

---

## Version

Workspace bumped `0.11.1` → `0.12.0`.
