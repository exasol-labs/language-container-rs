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

