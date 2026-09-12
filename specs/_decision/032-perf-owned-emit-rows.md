# Decisions: perf-owned-emit-rows

## ADR: `UdfContext::emit` takes an owned row, with no slice-taking compatibility path

**ID:** emit-takes-owned-row
**Plan:** `perf-owned-emit-rows`
**Status:** Accepted

### Context

Every emitted string was copied four times in user space: `emit(&[Value])` cloned the
slice into the buffer, `to_proto` cloned each cell again into the proto string block,
prost encoded into a fresh `Vec`, and libzmq copied that `Vec` into a message of its own.
The engine side of the same hop copies once. Authors already hold the row by value at the
call site. `query_for_each` hands them an owned `Vec<Value>`, and a generated row is built
per call. The borrow bought nothing.

### Decision

`emit` takes `Vec<Value>`. `EmitBuffer::take_proto` (previously `to_proto`) moves each
`Value::String`'s buffer into the string block and empties the buffer. The encoded frame is
handed to libzmq as an owned `zmq::Message`, which adopts the buffer via
`zmq_msg_init_data` instead of copying it. One user-space copy of a cell remains: the
protobuf encode.

Only the string payload moves. Every other variant is formatted from a borrow, as before.
Moving each cell by value costs 10 % on the `strblock` shape (DECIMAL, DATE, TIMESTAMP, no
string cell), where a 32-byte `Value` move buys nothing.

The slice form is removed rather than kept as a slower default, and
`EXA_UDF_ABI_VERSION` is bumped 7 → 8 so a stale `.so` fails at load time instead of
passing a slice where the host reads a `Vec`. Every UDF rebuilds and changes its `emit`
call sites once.

### Options Considered

| Option | Verdict |
|--------|---------|
| Owned `Vec<Value>`, slice form removed | ✓ Chosen — the copy disappears for every author, and there is no slow path left to fall into |
| Keep `emit(&[Value])`, add `emit_owned` | ✗ Rejected — two methods, and the copying one stays the obvious default |
| Keep the slice and intern strings host-side | ✗ Rejected — solves nothing for the common single-use string and adds a lookup per cell |

### Consequences

A breaking SDK change for every UDF crate: `ctx.emit(&[a, b])` becomes
`ctx.emit(vec![a, b])`, and a `query_for_each` callback forwards its row with
`|row| ctx.emit(row)`. `take_proto` leaves the buffer empty, so the flush sites no longer
call `clear()` separately.

Tier 1, interleaved base/change pair, `quick`: `scalar_emits_gen` row cells
−8.5 % native, −6.0 % strblock, −21.6 % varchar, −32.5 % wide; `scalar_emits_passthrough`
−8.2 %. `full` confirms the string shapes over two runs: varchar_row −21.5 / −21.1 %,
varchar_batch −10.4 / −9.5 %, wide_row −22.2 / −23.3 %. The Arrow batch cells do not
resolve on this harness. Unchanged code swung ±10 % run to run. The `MT_EMIT` counters
(messages, rows, bytes per cell) are byte-identical to the baseline.

## ADR: A retried `send` re-encodes rather than re-queueing the frame

**ID:** retried-send-re-encodes
**Plan:** `perf-owned-emit-rows`
**Status:** Accepted

### Context

`send` retries transient `EAGAIN` until the backstop. Handing libzmq an owned message
makes the retry unsound as written: `zmq_msg_send` failure drops the message and frees the
buffer, so the same frame cannot be queued a second time.

### Decision

Encode inside the retry closure. The happy path encodes once; a retry pays one extra
encode per poll interval.

### Options Considered

| Option | Verdict |
|--------|---------|
| Re-encode inside the retry loop | ✓ Chosen — zero-copy on the path that matters, retry semantics unchanged |
| Poll `POLLOUT` first, then send once | ✗ Rejected — an `EAGAIN` after a ready poll would have nothing left to retry, turning a stall into a VM abort |
| Keep `send(&buf)` and accept the libzmq copy | ✗ Rejected — a copy of the whole 4 MB frame on every `MT_EMIT` |

### Consequences

`send` logs `encoded_len()` rather than the buffer length; the retry path is otherwise
unchanged and still bounded by `MAX_TOTAL_WAIT`.
