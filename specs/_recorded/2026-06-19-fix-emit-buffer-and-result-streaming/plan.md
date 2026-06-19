# Plan: fix-emit-buffer-and-result-streaming

## Summary

Bound peak memory in two unbounded runtime paths: `ctx.emit` now buffers rows and flushes an `MT_EMIT` at a 4,000,000-byte wire threshold (and once more at end of `run`), and connect-back gains a streaming `query_for_each` that reads the result set one Arrow batch at a time instead of materializing it whole.

## Design

### Context

Two runtime paths accumulate an entire dataset in memory before handing anything to the next stage, so a UDF that emits or reads table-scale data OOMs:

1. **Emit:** `run_batch` builds one `EmitBuffer` per input batch and `consume_input` serializes the whole thing into a single `MT_EMIT` after the UDF's `run` returns. There is no size check; a UDF emitting millions of rows per input batch grows the buffer without bound.
2. **Connect-back:** `ExaConnection::query` (and the runtime override) call exarrow-rs `Connection::query`, which internally calls `ResultSet::fetch_all` — collecting every `RecordBatch` into a `Vec` before returning. A table-scale `SELECT` materializes the full result set.

The C++ SLC reference flushes its emit buffer whenever the accumulated message size reaches `SWIG_MAX_VAR_DATASIZE = 4_000_000` bytes (4 million bytes, not 4 MiB), and explicitly flushes once at end of run; it streams connect-back reads one fetch at a time. This plan mirrors both.

- **Goals**
  - Cap emit-buffer residency at ~4,000,000 bytes by flushing mid-run when a running byte estimate crosses `EMIT_BUFFER_LIMIT_BYTES = 4_000_000` (4 million bytes, not 4 MiB — matches `SWIG_MAX_VAR_DATASIZE`), and flush the tail at end of `run`.
  - Stream connect-back result sets one Arrow batch at a time, dropping each batch before fetching the next, so peak memory is one batch rather than the whole result.
  - Keep arrow types off the FFI boundary: the new streaming API yields owned `Vec<Value>` rows, never `RecordBatch`.
- **Non-Goals**
  - Splitting a single row across `MT_EMIT` frames. A row whose serialized size alone exceeds 4,000,000 bytes still flushes as one oversized `MT_EMIT`; only the protocol's 2 GB per-value limit remains as a hard ceiling.
  - Changing the wire protocol, the `ExascriptTableData` layout, or the connect-back session/transaction semantics.
  - Streaming the *input* batch path (`InputRowSet`) — the DB already controls input batch sizing on the wire.

### Decision

#### Architecture

```
                 emit path (Part 1)                       connect-back path (Part 2)

UDF run()                                       UDF run()
  └─ ctx.emit(row) ─▶ HostContextBridge::emit      └─ conn.query_for_each(sql, f)
        push row into EmitBuffer                        │
        if buf.should_flush():                          ▼
          (flusher closure) ──▶ MT_EMIT          RuntimeExaConnection::query_for_each
            serialize, send, await ack,                 Connection::execute(sql) ─▶ ResultSet
            buf.clear()                                 into_iterator() ─▶ ResultSetIterator
  (run returns)                                         loop on connect-back runtime:
consume_input: final MT_EMIT for the tail               fetch one RecordBatch
                                                          record_batch_to_rows(batch)
                                                          f(row) per row; drop batch
```

Both parts reuse the existing pattern of threading a closure into `HostContextBridge` (today only `conn_requester`). Part 1 adds a sibling `flusher` closure. Both closures need `&mut Protocol` + `&ZmqTransport`; they MUST share the **same** `RefCell<&mut Protocol>` so a mid-run emit flush and a `connection()` MT_IMPORT never hold overlapping mutable borrows. Calls are serial (the dispatch loop is blocked in `run_batch`), so one shared `RefCell` is sound.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Closure injected into bridge | `flusher` in `HostContextBridge`, mirroring `conn_requester` | The bridge owns the emit decision but must not own the ZMQ socket; the host wires the wire-flush behavior in as a callback |
| Single shared `RefCell<&mut Protocol>` | `run_batch` | `flusher` and `conn_requester` both need `&mut proto`; one cell keeps the serial borrows non-overlapping |
| Running byte estimate, not re-serialize | `EmitBuffer::push` / `should_flush` | Keeps emit cost linear; re-serializing on every push to measure size would be quadratic |
| Default-method streaming on the trait | `ExaConnection::query_for_each` | A mock implementing only `query_arrow` streams correctly; `query` collects via the same path so the two cannot diverge |
| Drive iterator on the connect-back runtime | `RuntimeExaConnection::query_for_each` | `ResultSetIterator::next_batch` calls `Handle::try_current()`; the fetch must run inside the runtime's `block_on` so a current handle exists |

#### Key interfaces

- `EmitBuffer`: add `byte_estimate: usize`; `push` updates it; new `should_flush(&self) -> bool` (`byte_estimate >= EMIT_BUFFER_LIMIT_BYTES`); `clear` resets it.
- `const EMIT_BUFFER_LIMIT_BYTES: usize = 4_000_000;` in `rowset.rs`.
- `type EmitFlusher<'a> = Box<dyn FnMut(&EmitBuffer) -> Result<(), UdfError> + 'a>;` threaded into `HostContextBridge::new` (feature-independent — emit flushing is not gated on `connect-back`).
- `ExaConnection::query_for_each<F: FnMut(Vec<Value>) -> Result<(), UdfError>>(&mut self, sql: &str, f: F) -> Result<(), UdfError>` (default impl over `query_arrow`).
- `record_batch_to_rows(&RecordBatch) -> Result<Vec<Vec<Value>>, UdfError>` in `exasol-udf-sdk::connect_back`; `record_batches_to_rows` re-expressed in terms of it.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Thread a `flusher` closure into the bridge | Give `EmitBuffer` an `Option<Box<dyn FnMut>>` field it owns | The bridge already threads `conn_requester` the same way; keeping the socket-touching closure out of `EmitBuffer` keeps `EmitBuffer` a pure data type that is trivially unit-testable |
| One shared `RefCell<&mut Protocol>` for both closures | Two separate `RefCell`s, or `Rc<RefCell<>>` | Two cells over the same `&mut` is unsound; calls are serial so one cell suffices and is simplest |
| Approximate byte estimate per value | Serialize-then-measure each push; track exact proto size | Exact size needs full serialization (quadratic); an approximation that sums per-`Value` byte costs is O(1) per push and only needs to be close enough to bound memory |
| `query_for_each` as default trait method | A separate streaming trait; a free function | A default method keeps every existing `ExaConnection` impl (including mocks) working unchanged and lets `query` delegate to it |
| Drive exarrow streaming via `execute` + `into_iterator` | Add a streaming method to exarrow-rs upstream | exarrow-rs 0.12.7 already exposes `Connection::execute -> ResultSet`, `ResultSet::into_iterator -> ResultSetIterator`; no upstream change needed |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| runtime/dispatch-run-loop | CHANGED | `runtime/dispatch-run-loop/spec.md` |
| runtime/connect-back | CHANGED | `runtime/connect-back/spec.md` |
| sdk/connect-back | CHANGED | `sdk/connect-back/spec.md` |

## Dependencies

- exarrow-rs 0.12.7 (already pinned): uses `Connection::execute`, `ResultSet::into_iterator`, `ResultSetIterator` (per-batch fetch). No version bump.

## Implementation Tasks

1. **EmitBuffer byte accounting** (`crates/exa-udf-runtime/src/rowset.rs`)
   1.1 Add `const EMIT_BUFFER_LIMIT_BYTES: usize = 4_000_000;`.
   1.2 Add `byte_estimate: usize` to `EmitBuffer`; implement a per-`Value` byte-cost helper and update `byte_estimate` in `push`. [expert]
   1.3 Add `should_flush(&self) -> bool` and reset `byte_estimate` in `clear`.
   1.4 Unit tests: byte estimate is monotonic; `should_flush` flips at the threshold; `clear` resets the estimate; a single oversized row makes `should_flush` true.

2. **Flusher closure threading** (`crates/exa-udf-runtime/src/rowset.rs`, `dispatch.rs`)
   2.1 Add `EmitFlusher<'a>` type alias and a `flusher` field to `HostContextBridge`; accept it in `new` and `with_connection`. [expert]
   2.2 In `HostContextBridge::emit`: push the row, then if `emit_buf.should_flush()` invoke the flusher and propagate its error.
   2.3 In `run_batch`: build the `flusher` closure that serializes `EmitBuffer::to_proto`, sends `MT_EMIT`, awaits the emit ack, and clears the buffer — sharing the **same** `RefCell<&mut Protocol>` as `conn_requester`. [expert]
   2.4 Keep the existing end-of-`run` tail flush in `consume_input` (it now sends only the residual rows).
   2.5 Update all `HostContextBridge::new`/`with_connection` call sites and the `make_bridge` test helper to pass a flusher (a no-op or recording closure in tests).

3. **Single-batch converter** (`crates/exasol-udf-sdk/src/connect_back.rs`)
   3.1 Add `record_batch_to_rows(&RecordBatch) -> Result<Vec<Vec<Value>>, UdfError>`; re-express `record_batches_to_rows` as a loop over it.
   3.2 Unit test: `record_batch_to_rows` on one batch equals the per-batch slice of `record_batches_to_rows`.

4. **query_for_each trait API** (`crates/exasol-udf-sdk/src/connect_back.rs`)
   4.1 Add `query_for_each<F: FnMut(Vec<Value>) -> Result<(), UdfError>>` to `ExaConnection` with a default impl over `query_arrow` + `record_batch_to_rows`, stopping on the first callback error.
   4.2 Re-express the default `query` to call `query_for_each` and collect into `Vec<Vec<Value>>`.
   4.3 Unit tests (mock connection returning two batches): callback fires per row in batch-then-row order; `query` and `query_for_each` agree; callback error halts iteration.

5. **RuntimeExaConnection streaming override** (`crates/exa-udf-runtime/src/connect_back.rs`)
   5.1 Override `query_for_each`: `block_on(self.inner.execute(sql))`, `into_iterator()`, then loop fetching one batch at a time on the connect-back runtime, convert via `record_batch_to_rows`, call `f` per row, drop the batch before the next fetch. [expert]
   5.2 Wrap the fetch+convert loop in `catch_unwind`, mapping panics and `QueryError` to `UdfError::ConnectBack`; satisfy the iterator's `Handle::try_current` requirement by driving each fetch inside the runtime context. [expert]
   5.3 Re-express the `query` override to delegate to `query_for_each` (collect rows), keeping the arrow→Value conversion in the runtime's arrow context.

6. **Integration test fixtures** (`test-udfs/`)
   6.1 New `test-udfs/emit-bulk`: a SET UDF that emits N rows large enough that the total crosses 4,000,000 bytes within one input batch (forces ≥1 mid-run flush + a tail flush). Build as `libemit_bulk.so`.
   6.2 New `test-udfs/connect-back-stream`: a SET UDF that `query_for_each`s a SELECT returning many rows (table-scale) and emits an aggregate (e.g. row count / checksum) proving every row was streamed.

7. **Integration tests** (`crates/it/tests/db_roundtrip.rs`)
   7.1 Add `emit_bulk_flushes_multiple_batches`: register `libemit_bulk.so`, invoke over the live container, assert the full expected row count returns (all flushes landed, none lost).
   7.2 Add `connect_back_stream_reads_all_rows`: seed a multi-row committed table, invoke `libconnect_back_stream.so`, assert the aggregate equals the seeded data (every streamed row processed). Hard assertion across the version matrix, following the existing connect-back scenarios.

8. **Dead code / cleanup**
   8.1 Confirm `consume_input`'s post-run emit block now only handles the tail; adjust its comment to reflect mid-run flushing.
   8.2 Confirm no caller still relies on `EmitBuffer` growing unbounded.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 (EmitBuffer accounting), Task 3 (single-batch converter) |
| Group B | Task 2 (flusher threading), Task 4 (query_for_each trait) |
| Group C | Task 5 (runtime streaming override), Task 6 (fixtures) |
| Group D | Task 7 (integration tests), Task 8 (cleanup) |

Sequential dependencies:
- Group A → Group B (Task 2 needs `should_flush`/`clear`; Task 4 needs `record_batch_to_rows`)
- Group B → Group C (Task 5 overrides the trait method defined in Task 4)
- Group C → Group D (integration tests exercise the fixtures and the streaming override)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Comment | `crates/exa-udf-runtime/src/dispatch.rs` (`consume_input` emit block, run_udf doc) | Now describes mid-run + tail flush instead of "single MT_EMIT per batch" |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| dispatch-run-loop / Set/EMITS dispatch emits multiple rows across batches | Integration | `crates/it/tests/db_roundtrip.rs` | `emit_bulk_flushes_multiple_batches` |
| dispatch-run-loop / A single emitted row larger than the flush threshold is sent on its own | Unit | `crates/exa-udf-runtime/src/rowset.rs` (tests) | `oversized_single_row_flushes_alone` |
| dispatch-run-loop / EmitBuffer tracks a running byte estimate and reports when to flush | Unit | `crates/exa-udf-runtime/src/rowset.rs` (tests) | `emit_buffer_byte_estimate_and_should_flush` |
| connect-back / query_for_each streams the result set one batch at a time | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_stream_reads_all_rows` |
| connect-back / Connect-back query returns Arrow batches to the UDF | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` (existing, still green via the re-expressed `query`) |
| sdk/connect-back / ExaConnection trait is defined behind the connect-back feature | Unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | `exa_connection_trait_surface_compiles` |
| sdk/connect-back / query_for_each default streams rows to the callback on a mock connection | Unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | `query_for_each_default_streams_rows` |
| sdk/connect-back / record_batch_to_rows converts a single batch without collecting the whole result | Unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | `record_batch_to_rows_matches_multibatch` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| runtime/dispatch-run-loop | `cargo test -p it --features integration db_roundtrip_all_scenarios -- --nocapture` (with the `emit-bulk` UDF) then read `[it] scenario emit_bulk ok` | The bulk-emit query returns the full expected row count; harness logs show multiple `MT_EMIT` flushes for one input batch |
| runtime/connect-back | `cargo test -p it --features integration db_roundtrip_all_scenarios -- --nocapture` | `[it] scenario connect_back_stream ok`; aggregate over the streamed result equals the seeded table, with no OOM and no SIGABRT |
| sdk/connect-back | `cargo test -p exasol-udf-sdk --features connect-back` | `query_for_each_default_streams_rows` and `record_batch_to_rows_matches_multibatch` pass |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Build binary | `cargo build --release -p exaudfclient` | Exit 0 |
| Unit/integration test | `cargo test` | 0 failures |
| E2E (DB) | `cargo test -p it --features integration` | 0 failures (Exasol Docker container required; fails, not skips, if unavailable) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
