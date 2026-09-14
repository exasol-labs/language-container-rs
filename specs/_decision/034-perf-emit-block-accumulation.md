# Decisions: perf-emit-block-accumulation

## ADR: `EmitBuffer` accumulates the proto blocks, not `Vec<Value>` rows

**ID:** emit-buffer-accumulates-proto-blocks
**Plan:** `perf-emit-block-accumulation`
**Status:** Accepted

### Context

`EmitBuffer` buffered `Vec<Vec<Value>>` and packed the type blocks only in `take_proto`, so
`push_batch` could not append to it. It flushed whatever the row path had buffered, encoded
each full 4 MB `RecordBatch::slice` separately, and pivoted the trailing remainder back
through `Vec<Value>`. A batch below 4 MB never produces a slice, so the 8192-row batches
callers actually emit were encoded entirely by that pivot — which is why `emit_batch`
measured slower than `ctx.emit` for string-block and wide rows.

### Decision

The buffer holds the type blocks themselves. `push` packs a row as it arrives, `push_batch`
appends a downcast batch row by row into the same blocks, and `take_proto` is a `mem::take`.
There is no tail case, no `RecordBatch::slice`, and no `Vec<Value>` on either path.

A batch is costed in O(columns) — fixed-width columns from their null count, variable-width
from the offset buffer's span — and only a batch whose total could reach the 4 MB threshold
pays for a per-row cost vector.

### Options Considered

| Option | Verdict |
|--------|---------|
| Accumulate the blocks; append batch rows into them | ✓ Chosen — removes the pivot that dominated the sub-4 MB batch, and `emit`/`emit_batch` stop displacing each other |
| Keep the row buffer, route only the tail through the columnar encoder | ✗ Rejected — measured a third of the win and keeps two encoders that must agree byte for byte |
| Keep `RecordBatch::slice` for the over-threshold case | ✗ Rejected — a second encoder for a case the row loop already handles at the same row granularity |

### Consequences

`push`/`push_costed` take the declared output columns (the bridge already holds them) and
`take_proto` takes none. Buffered memory is the wire estimate rather than a `Vec<Value>`
copy of it, and interleaved `emit` and `emit_batch` no longer force an undersized
`MT_EMIT`. Encoding now happens on the `emit` call rather than at flush; the row is still
walked once, because the bridge's validation pass supplies the byte cost.

Tier 1 `quick`, against one baseline: every cell improved or held, none regressed —
`scalar_emits_passthrough/native` -42 %, `scalar_emits_gen` native_row -33 %, native_batch
-38 %, strblock_row -17 %, varchar_row -15 %, varchar_batch -12 %, wide_row -8 %;
`set_emits/native_batch_g1` -37 %; `scalar_returns` native -14 %, strblock -13 %. The
`MT_EMIT` counters (messages, rows, bytes per cell, `bytes/row`) are unchanged.

Tier 2 `quick`, alternating base/change over one docker-db 2026.1.1: `scalar_emits_gen`
native_batch -25.0 %, native_row -24.5 %, varchar_batch -10.9 %, varchar_row -10.0 %,
`scalar_emits_pt` -17.2 %, `set_gen` native_batch -25.5 %, varchar_batch -13.8 %, wide_row
-7.2 %; no UDF cell regressed. Net of its `_noemit` twin the strblock batch path is 6.5 %
slower than the row path, down from 12.3 %: the `Vec<Value>` pivot is gone, but the string
block still allocates per cell and prost still copies it into the frame.

## ADR: The string block stays `repeated string`

**ID:** string-block-stays-repeated-string
**Plan:** `perf-emit-block-accumulation`
**Status:** Accepted

### Context

`repeated string` gives prost a `Vec<String>`: one heap allocation per NUMERIC, DATE and
TIMESTAMP cell the encoder formats. Protobuf encodes `string` and `bytes` identically — tag
2, length-delimited — so declaring the field `bytes` and handing prost `Bytes` views of one
arena per flush would remove that allocation without changing a wire byte.

### Decision

Keep `repeated string`. The arena variant was implemented and measured: it is slower on
every shape whose output uses the string block.

`Vec<Bytes>` needs one `Bytes` per cell, and each one costs a refcount pair on the shared
arena. That is roughly what the `String` allocation it replaces costs, while an owned
`Value::String` — which today moves into the block for free — has to pay it too. The
allocation is only worth removing together with the copy prost still makes into the frame,
which needs a hand-written encoder for `exascript_table_data`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep `repeated string` | ✓ Chosen — the per-cell allocation is not the bottleneck the block accumulation was, and removing it this way costs more than it saves |
| Declare `bytes`, slice one arena per flush | ✗ Rejected — measured on Tier 1 `quick` against the same baseline: `wide_row` +35 %, `strblock_batch` +31 %, `varchar_row` +19 %, `strblock_row` +13 % |
| Hand-write the `exascript_table_data` wire bytes | ✗ Rejected here — a second protobuf encoder to keep byte-identical, for an increment on top of this one |

### Consequences

The vendored `zmqcontainer.proto` stays byte-identical to upstream. `InputRowSet::from_proto`
borrows each string-block cell instead of cloning it, which was the one ingest-side change
worth keeping from the experiment.
