# Decisions: fix-emit-buffer-and-result-streaming

## ADR: Read connect-back results via execute and fetch_all inside one block_on

**ID:** stream-connect-back-execute-resultset-iterator
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

`ResultSetIterator::next_batch` in exarrow-rs calls `handle.block_on()`. On the single-thread `current_thread` Tokio runtime, calling it from inside an outer `block_on` deadlocks.

### Decision

`RuntimeExaConnection::query_for_each` calls `Connection::execute(sql)` and `fetch_all()` inside one `block_on`. It then iterates the owned `Vec<RecordBatch>`, converting and dropping each batch before the next, so the whole result never exists as `Value` rows.

### Options Considered

| Option | Verdict |
|--------|---------|
| `execute` and `fetch_all` in one `block_on`, iterate owned batches | ✓ Chosen: no deadlock on the `current_thread` runtime |
| Drive `ResultSetIterator::next_batch` per batch | ✗ Rejected: deadlocks on a single-thread runtime, and it keeps every batch it returns |

### Consequences

- All batches are fetched before the first callback, so peak memory is the whole result in Arrow form plus one batch of `Value` rows.
- Bounding memory to one batch needs an exarrow-rs API that fetches one batch at a time asynchronously.
