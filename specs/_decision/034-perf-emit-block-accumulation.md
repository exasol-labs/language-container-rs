# Decisions: perf-emit-block-accumulation

## ADR: `EmitBuffer` accumulates the proto blocks, not `Vec<Value>` rows

**ID:** emit-buffer-accumulates-proto-blocks
**Plan:** `perf-emit-block-accumulation`
**Status:** Accepted

### Context

Buffering `Vec<Vec<Value>>` and packing the type blocks only in `take_proto` forces `push_batch` through a `Vec<Value>` pivot. Batches below 4 MB never reach the slice path, so the pivot encodes them all.

### Decision

The buffer holds the type blocks themselves. `push` packs a row as it arrives, `push_batch` appends a downcast batch row by row into the same blocks, and `take_proto` is a `mem::take`. There is no `RecordBatch::slice` and no `Vec<Value>` on either path.

A batch is costed in O(columns): fixed-width columns from their null count, variable-width columns from the offset buffer's span. Only a batch whose total could reach the 4 MB threshold pays for a per-row cost vector.

### Options Considered

| Option | Verdict |
|--------|---------|
| Accumulate the blocks, append batch rows into them | ✓ Chosen |
| Keep the row buffer, route the tail through a columnar encoder | ✗ Keeps two encoders that must agree byte for byte |
| Keep `RecordBatch::slice` for over-threshold batches | ✗ Second encoder for a case the row loop already handles |

### Consequences

`push`/`push_costed` take the declared output columns, and `take_proto` takes none. Encoding happens on the `emit` call, and the row is walked once because the bridge's validation pass supplies the byte cost. Interleaved `emit` and `emit_batch` do not force an undersized `MT_EMIT`.

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
