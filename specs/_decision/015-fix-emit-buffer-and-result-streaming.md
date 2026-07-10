# Decisions: fix-emit-buffer-and-result-streaming

## ADR: Thread an emit-flusher closure into HostContextBridge, mirroring conn_requester

**ID:** emit-flusher-closure-host-context-bridge
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

The prior `ctx.emit` accumulated rows in `EmitBuffer` indefinitely — there was no size check, so a UDF emitting millions of rows per input batch would grow the buffer without bound. A flush was only sent after `run()` returned, so peak memory was the full batch output. A mechanism was needed to flush mid-run when the buffer crossed a threshold, matching the C++ SLC reference (`SWIG_MAX_VAR_DATASIZE = 4_000_000`).

### Decision

Add a `flusher: EmitFlusher` closure to `HostContextBridge`, threaded in by `run_batch` exactly like the existing `conn_requester`. `HostContextBridge::emit` pushes the row then invokes the flusher when `should_flush()` is true.

### Options Considered

| Option | Verdict |
|--------|---------|
| Thread a `flusher` closure into the bridge (mirroring `conn_requester`) | ✓ Chosen — reuses the established closure-injection pattern; keeps the ZMQ socket out of `EmitBuffer`; `EmitBuffer` stays a pure, trivially-unit-testable data type |
| Give `EmitBuffer` an owned `Option<Box<dyn FnMut>>` flush callback | ✗ Rejected — pulls socket-touching behavior into what should stay a pure data type |

### Consequences

The bridge now carries two closures: `conn_requester` (connect-back) and `flusher` (emit). Both closures are injected by `run_batch` and must share the same `RefCell<&mut Protocol>` (see ADR-042). `EmitBuffer` remains a pure data structure with no I/O dependencies.

## ADR: Both bridge closures share one RefCell<&mut Protocol>

**ID:** bridge-closures-share-one-refcell-protocol
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

The emit flusher (ADR-041) and the existing `conn_requester` both need `&mut Protocol` to send wire messages. In `run_batch`, both closures are live at the same time. A sound mechanism was needed to share access to the single `&mut Protocol`.

### Decision

The emit flusher and the connect-back `conn_requester` borrow the same `RefCell<&mut Protocol>` in `run_batch`. Calls are strictly serial because the dispatch loop is blocked inside `run_batch` awaiting the UDF function return.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single shared `RefCell<&mut Protocol>` for both closures | ✓ Chosen — calls are serial; one cell yields non-overlapping borrows; simplest sound option |
| Two separate `RefCell`s over the same `&mut Protocol` | ✗ Rejected — aliasing a unique borrow is unsound |
| `Rc<RefCell<>>` | ✗ Rejected — unnecessary heap/refcount overhead for a strictly-serial call pattern |

### Consequences

A single `RefCell<&mut Protocol>` is shared between `flusher` and `conn_requester`. Simultaneous borrows cannot occur because the dispatch loop is blocked during UDF execution, so the serial constraint is structural rather than enforced at compile time.

## ADR: query_for_each as a default trait method; query delegates to it

**ID:** query-for-each-default-trait-method
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

`ExaConnection::query` (and the runtime override) called `Connection::query` which internally called `fetch_all`, materializing the entire result set into a `Vec<RecordBatch>` before returning. A streaming API was needed so UDFs could process result sets without holding the entire result in memory. The design had to keep every existing `ExaConnection` impl (including test mocks) compiling unchanged.

### Decision

Add `query_for_each<F: FnMut(Vec<Value>) -> Result<(), UdfError>>` to `ExaConnection` with a default implementation over `query_arrow` that converts each batch's rows and invokes the callback. Re-express the default `query` to collect via `query_for_each`, sharing one code path. The runtime overrides `query_for_each` with the true streaming path; `query` automatically delegates to that override.

### Options Considered

| Option | Verdict |
|--------|---------|
| `query_for_each` as a default trait method; `query` delegates to it | ✓ Chosen — backward compatible; every existing impl works unchanged; `query` and streaming cannot diverge; one conversion code path |
| A separate streaming trait | ✗ Rejected — breaks existing mock impls; requires double-implementation for the runtime |
| A free function | ✗ Rejected — does not allow the runtime override to automatically fix `query`'s materialization behavior |

### Consequences

Every `ExaConnection` impl that provides only `query_arrow` automatically gets streaming behavior via the default. `query` is no longer materialization-primary — it delegates to `query_for_each` and collects. A mock that implements only `query_arrow` passes both `query` and `query_for_each` tests without change.

## ADR: Stream connect-back via execute + ResultSetIterator with fetch_all fallback due to current_thread constraint

**ID:** stream-connect-back-execute-resultset-iterator
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

`RuntimeExaConnection::query_for_each` needed to stream one `RecordBatch` at a time rather than materializing the full result. exarrow-rs 0.12.7 exposes `Connection::execute -> ResultSet`, `ResultSet::into_iterator -> ResultSetIterator`, and `ResultSetIterator::next_batch`. The plan specified driving `next_batch` in a loop. During implementation, a `current_thread` Tokio runtime constraint was discovered: `next_batch` calls `Handle::try_current()` then `handle.block_on(fetch_next_batch())`. Calling `handle.block_on()` from within an outer `Runtime::block_on(async{})` deadlocks — the single thread is already occupied.

### Decision

`RuntimeExaConnection::query_for_each` uses `Connection::execute(sql)` followed by `fetch_all()` inside one `block_on`, then iterates the owned `Vec<RecordBatch>` with `into_iter()`, dropping each batch before processing the next. This eliminates the double-copy peak (Arrow + Value simultaneously) of the old `query` implementation, though it does not achieve true per-batch server streaming. Per-batch streaming requires a future exarrow-rs API that can be polled without a nested `handle.block_on` call.

### Options Considered

| Option | Verdict |
|--------|---------|
| `execute` + `fetch_all` inside one `block_on`, iterate owned batches | ✓ Chosen — eliminates the double-copy peak; no deadlock; works with the current `current_thread` runtime; no exarrow-rs version bump |
| Drive `ResultSetIterator::next_batch` per-batch on the `current_thread` runtime | ✗ Rejected — deadlocks: `next_batch` calls `handle.block_on()` from within an outer `block_on` on a single-thread runtime |
| Add a streaming API to exarrow-rs upstream | ✗ Rejected — no version bump needed; 0.12.7 already exposes the closest available primitive |

### Consequences

True per-batch server streaming (dropping batch _N_ before fetching batch _N+1_ from the server) is not achieved — all batches are fetched before row-by-row processing begins. However, the per-batch drop loop eliminates the simultaneous Arrow + Value peak of the prior `query` implementation. Per-batch server streaming requires a future exarrow-rs API change. This deviation from the plan is documented in the verification report.
