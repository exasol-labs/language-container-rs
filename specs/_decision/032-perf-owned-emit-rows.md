# Decisions: perf-owned-emit-rows

## ADR: `UdfContext::emit` takes an owned row, with no slice-taking compatibility path

**ID:** emit-takes-owned-row
**Plan:** `perf-owned-emit-rows`
**Status:** Accepted

### Context

Authors already hold the row by value at the call site, and a `query_for_each` callback receives an owned row.

### Decision

`emit` takes `Vec<Value>`. The encoded frame goes to libzmq as an owned `zmq::Message` that adopts the buffer via `zmq_msg_init_data` instead of copying it.

The slice form does not exist, and `EXA_UDF_ABI_VERSION` rejects a stale `.so` at load time.

### Options Considered

| Option | Verdict |
|--------|---------|
| Owned `Vec<Value>`, no slice form | ✓ Chosen |
| Keep `emit(&[Value])`, add `emit_owned` | ✗ Two methods, and the copying one stays the obvious default |
| Keep the slice and intern strings host-side | ✗ Adds a lookup per cell and does not help single-use strings |

### Consequences

- Every UDF crate calls `ctx.emit(vec![a, b])`, and a `query_for_each` callback forwards its row with `|row| ctx.emit(row)`.
- The buffer copies each string cell into the wire-form string block (`string-block-bytes-hand-encoded-emit`), so the owned row saves no copy of a cell.
- `send` encodes the frame inside the retry closure, so a transient `EAGAIN` retry re-encodes, because a failed `zmq_msg_send` frees the owned buffer.
