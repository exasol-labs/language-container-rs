# Decisions: fix-emit-buffer-and-result-streaming

## ADR: Stream connect-back via execute and fetch_all inside one block_on

**ID:** stream-connect-back-execute-resultset-iterator
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

`ResultSetIterator::next_batch` in exarrow-rs calls `handle.block_on()`. On the single-thread `current_thread` Tokio runtime, calling it from inside an outer `block_on` deadlocks.

### Decision

`RuntimeExaConnection::query_for_each` calls `Connection::execute(sql)` and `fetch_all()` inside one `block_on`. It then iterates the owned `Vec<RecordBatch>` and drops each batch before the next. Arrow batches and `Value` rows never coexist for the whole result.

### Options Considered

| Option | Verdict |
|--------|---------|
| `execute` and `fetch_all` in one `block_on`, iterate owned batches | ✓ Chosen: no deadlock on the `current_thread` runtime |
| Drive `ResultSetIterator::next_batch` per batch | ✗ Rejected: deadlocks on a single-thread runtime |

### Consequences

All batches are fetched before row processing starts, so the server does not stream per batch.
