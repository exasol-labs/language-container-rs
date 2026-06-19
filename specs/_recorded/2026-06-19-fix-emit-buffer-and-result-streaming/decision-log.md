# Decision Log: fix-emit-buffer-and-result-streaming

Date: 2026-06-19

## Interview

No clarifying Q&A round was needed: the architect stated the constraints directly. Recorded verbatim/paraphrased below.

**Q (implicit):** What is the emit flush threshold?
**A:** "4 MB ist das Limit" → `EMIT_BUFFER_LIMIT_BYTES = 4_000_000` bytes (4 million bytes, matching `SWIG_MAX_VAR_DATASIZE` in the C++ SLC reference, not 4 MiB).

**Q (implicit):** Must the buffer flush at end of run even if under threshold?
**A:** "beim buffern ist auch wichtig, das man flushed, wenn die Run Methode durch ist" → yes; flush the tail at end of `run` regardless of size.

**Q (implicit):** Should each `emit` send an `MT_EMIT`?
**A:** "ein emit muss nicht gleich eine emit message senden, sondern sollte vorher bis 4MB sammeln und dann senden" → buffer rows; flush only at the threshold or at end of run.

**Q (implicit):** How should connect-back result sets be read?
**A:** "du musst resultset in batches lesen und dann gleich emitten" → stream one Arrow batch at a time, convert to rows, hand off, drop, repeat.

**Q (implicit):** What about a row that alone exceeds 4 MB?
**A:** "aber die limitierung das eine row max 2GB haben kann, wirst du nicht los" → a single oversized row still flushes as one `MT_EMIT`; rows are never split; only the 2 GB per-value protocol limit remains.

## Design Decisions

### [1] Thread an emit-flusher closure into HostContextBridge

- **Decision:** Add a `flusher: EmitFlusher` closure to `HostContextBridge`, threaded in by `run_batch` exactly like the existing `conn_requester`. `emit` pushes the row then invokes the flusher when `should_flush()` is true.
- **Alternatives:** Give `EmitBuffer` an owned `Option<Box<dyn FnMut>>` flush callback. Rejected — it pulls socket-touching behavior into what should stay a pure, trivially-unit-testable data type.
- **Rationale:** Reuses the bridge's established closure-injection pattern; keeps the ZMQ socket out of `EmitBuffer`.
- **Promotes to ADR:** yes

### [2] Both bridge closures share one RefCell<&mut Protocol>

- **Decision:** The emit flusher and the connect-back `conn_requester` borrow the **same** `RefCell<&mut Protocol>` in `run_batch`.
- **Alternatives:** Two separate `RefCell`s over the same `&mut Protocol` (unsound — aliasing a unique borrow); `Rc<RefCell<>>` (unnecessary heap/refcount).
- **Rationale:** Calls are strictly serial because the dispatch loop is blocked inside `run_batch`, so a single cell yields non-overlapping borrows and is the simplest sound option.
- **Promotes to ADR:** yes

### [3] Approximate, O(1) byte estimate rather than exact serialized size

- **Decision:** `EmitBuffer::push` increments `byte_estimate` by a per-`Value` byte-cost approximation; `should_flush` compares against `EMIT_BUFFER_LIMIT_BYTES`.
- **Alternatives:** Serialize the buffer on each push to measure exact size. Rejected — quadratic emit cost.
- **Rationale:** The threshold only needs to bound memory, not be exact; an O(1) running total keeps emit linear.
- **Promotes to ADR:** no

### [4] query_for_each as a default trait method; query delegates to it

- **Decision:** Add `query_for_each<F: FnMut(Vec<Value>)>` to `ExaConnection` with a default over `query_arrow`; re-express the default `query` to collect via `query_for_each`. The runtime overrides `query_for_each` with the true streaming path and `query` delegates to that override.
- **Alternatives:** A separate streaming trait or free function. Rejected — a default method keeps every existing impl (incl. mocks) compiling and prevents `query`/streaming divergence.
- **Rationale:** One conversion code path; backward compatible.
- **Promotes to ADR:** yes

### [5] Stream via exarrow-rs execute + ResultSetIterator (no upstream change)

- **Decision:** `RuntimeExaConnection::query_for_each` uses `Connection::execute(sql) -> ResultSet`, `into_iterator()`, and fetches one `RecordBatch` at a time on the dedicated connect-back runtime; it does NOT call `Connection::query`/`fetch_all`.
- **Alternatives:** Add a streaming API to exarrow-rs upstream. Rejected — 0.12.7 already exposes the needed iterator. Note: the architect's premise that `Connection::query` "returns batches as they arrive" was inaccurate — `query` calls `fetch_all`, materializing everything; the per-batch primitive lives on `ResultSet`/`ResultSetIterator`.
- **Rationale:** No dependency bump; the existing pinned exarrow-rs already supports per-batch fetch. `ResultSetIterator::next_batch` requires a current tokio handle, so each fetch is driven inside the runtime's `block_on` context.
- **Promotes to ADR:** yes

## Review Findings

All 6 correctness invariants confirmed by independent code review (2026-06-19).

**Deviation from plan [5]**: `RuntimeExaConnection::query_for_each` uses `fetch_all()` rather than `ResultSetIterator::next_batch()`. Root cause: on a `current_thread` Tokio runtime, `next_batch()` calls `Handle::try_current()` then `handle.block_on(fetch_next_batch())`. Calling `handle.block_on()` from within an outer `Runtime::block_on(async{})` deadlocks — the runtime's single thread is already occupied. Decision: use `fetch_all()` inside one `block_on`, then iterate the owned `Vec<RecordBatch>` with `into_iter()`, dropping each batch before processing the next. This eliminates the double-copy peak (Arrow + Value simultaneously) of the old `query` implementation, though it does not achieve true per-batch server streaming. Per-batch streaming requires a future exarrow-rs API change (streaming iterator callable without `Handle::block_on` re-entry).

**Format**: `cargo fmt` was run during review to fix formatting in `exasol-udf-sdk/tests/connect_back.rs`.
