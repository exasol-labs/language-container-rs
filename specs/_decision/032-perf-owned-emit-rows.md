# Decisions: perf-owned-emit-rows

## ADR: `UdfContext::emit` takes an owned row, with no slice-taking compatibility path

**ID:** emit-takes-owned-row
**Plan:** `perf-owned-emit-rows`
**Status:** Accepted

### Context

Borrowing a row slice copies each emitted string several times in user space. Authors already hold the row by value at the call site.

### Decision

`emit` takes `Vec<Value>`. `EmitBuffer::take_proto` moves each `Value::String`'s buffer into the string block and empties it. The encoded frame goes to libzmq as an owned `zmq::Message` that adopts the buffer via `zmq_msg_init_data`. The protobuf encode is the one remaining user-space copy of a cell.

Only the string payload moves. Every other variant is formatted from a borrow, because moving a 32-byte `Value` buys nothing for non-string cells. The slice form does not exist, and `EXA_UDF_ABI_VERSION` rejects a stale `.so` at load time.

### Options Considered

| Option | Verdict |
|--------|---------|
| Owned `Vec<Value>`, no slice form | ✓ Chosen |
| Keep `emit(&[Value])`, add `emit_owned` | ✗ Two methods, and the copying one stays the obvious default |
| Keep the slice and intern strings host-side | ✗ Adds a lookup per cell and does not help single-use strings |

### Consequences

Every UDF crate calls `ctx.emit(vec![a, b])`, and a `query_for_each` callback forwards its row with `|row| ctx.emit(row)`. `take_proto` leaves the buffer empty, so flush sites do not call `clear()`. `send` encodes the frame inside the retry closure, so a transient `EAGAIN` retry re-encodes, because a failed `zmq_msg_send` frees the owned buffer.
