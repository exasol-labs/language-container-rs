# Tasks: fix-emit-buffer-and-result-streaming

## Group A — Parallel foundations (no dependencies)

### A1: EmitBuffer byte accounting (rowset.rs)
- [x] 1.1 Add `const EMIT_BUFFER_LIMIT_BYTES: usize = 4_000_000;` in `rowset.rs`
- [x] 1.2 Add `byte_estimate: usize` to `EmitBuffer`; implement per-`Value` byte-cost helper; update `push` to increment it [expert]
- [x] 1.3 Add `should_flush(&self) -> bool` (`byte_estimate >= EMIT_BUFFER_LIMIT_BYTES`) and reset `byte_estimate` in `clear`
- [x] 1.4 Unit tests: estimate monotonic; `should_flush` flips at threshold; `clear` resets; single oversized row triggers flush

### A3: Single-batch converter (exasol-udf-sdk/src/connect_back.rs)
- [x] 3.1 Add `pub fn record_batch_to_rows(batch: &RecordBatch) -> Result<Vec<Vec<Value>>, UdfError>`; re-express `record_batches_to_rows` as a loop over it
- [x] 3.2 Unit test: `record_batch_to_rows` on one batch equals per-batch slice of `record_batches_to_rows`

## Group B — Build on Group A (sequential after A)

### B2: Flusher closure threading (rowset.rs + dispatch.rs)
- [x] 2.1 Add `EmitFlusher<'a>` type alias; add `flusher` field to `HostContextBridge`; accept it in `new` and `with_connection` [expert]
- [x] 2.2 In `HostContextBridge::emit`: push row, if `emit_buf.should_flush()` invoke the flusher and propagate error
- [x] 2.3 In `run_batch`: build `flusher` closure that serializes `to_proto`, sends `MT_EMIT`, awaits ack, clears buffer — sharing the same `RefCell<&mut Protocol>` as `conn_requester` [expert]
- [x] 2.4 Keep end-of-run tail flush in `consume_input` (now sends only residual rows)
- [x] 2.5 Update all `HostContextBridge::new`/`with_connection` call sites + `make_bridge` test helper (pass no-op or recording closure)

### B4: query_for_each trait API (exasol-udf-sdk/src/connect_back.rs)
- [x] 4.1 Add `query_for_each<F: FnMut(Vec<Value>) -> Result<(), UdfError>>` to `ExaConnection` as a default method over `query_arrow` + `record_batch_to_rows`; stop on first callback error
- [x] 4.2 Re-express default `query` to call `query_for_each` and collect into `Vec<Vec<Value>>`
- [x] 4.3 Unit tests (mock with two batches): callback fires per row in batch-then-row order; `query` and `query_for_each` agree; callback error halts iteration

## Group C — Build on Group B (sequential after B)

### C5: RuntimeExaConnection streaming override (exa-udf-runtime/src/connect_back.rs)
- [x] 5.1 Override `query_for_each`: drive `connection.execute(sql).await?.fetch_all().await?` inside `block_on`; iterate the resulting `Vec<RecordBatch>` with `into_iter()` (moves batches one-by-one), convert each via `record_batch_to_rows`, call `f` per row, drop batch before next [expert]
  - NOTE: `ResultSetIterator::next_batch()` calls `Handle::try_current()` then `handle.block_on()`. On our `current_thread` runtime this would deadlock if called from within `block_on(async{})`. Use `fetch_all()` + sequential `into_iter()` instead for safe implementation. True per-batch server-side streaming requires a future exarrow-rs API change.
- [x] 5.2 Wrap the entire operation in `catch_unwind`; map panics and `QueryError` to `UdfError::ConnectBack` [expert]
- [x] 5.3 Re-express the `query` override to delegate to `query_for_each` (collect rows)

### C6: Integration test fixtures (test-udfs/)
- [x] 6.1 New `test-udfs/emit-bulk`: SET UDF that emits N rows each with a 100-byte string value, so total > 4,000,000 bytes in one input batch (e.g. 50,000 rows × ~80 bytes ≈ 4 MB). Follow `test-udfs/set-filter` as template. UDF receives one input column (VARCHAR), emits same varchar back repeatedly (N times per input row) to trigger mid-run flush + tail flush.
- [x] 6.2 New `test-udfs/connect-back-stream`: SET UDF that calls `conn.query_for_each("SELECT ...", |row| { count += 1; Ok(()) })` on a seeded table and emits the final row count as a BIGINT. Follow `test-udfs/connect-back-query` as template.

## Group D — Build on Group C (sequential after C)

### D7: Integration tests (crates/it/tests/db_roundtrip.rs)
- [x] 7.1 Add `EMIT_BULK_LIB` const; upload in the bulk upload block; call `emit_bulk_flushes_multiple_batches` scenario; assert total emitted rows equals N × input_row_count (all flushes landed)
- [x] 7.2 Add `CB_STREAM_LIB` const; upload; seed a committed table with M rows; call `connect_back_stream_reads_all_rows` scenario; assert emitted row count == M

### D8: Cleanup
- [x] 8.1 Update doc comment on `consume_input` and `run_udf` in `dispatch.rs` to describe mid-run + tail flush instead of "single MT_EMIT per batch"
- [x] 8.2 Audit that no caller relies on `EmitBuffer` growing unbounded (check all `EmitBuffer::push` call sites)

## Verification
- [x] V1 `cargo build --release` — exit 0
- [x] V2 `cargo build --release -p exaudfclient` — exit 0
- [x] V3 `cargo test` — 0 failures
- [x] V4 `cargo test -p it --features integration` — 0 failures (Exasol 2026.1.0)
- [x] V5 `cargo clippy --all-targets --all-features -- -D warnings` — 0 errors
- [x] V6 `cargo fmt --check` — no changes
