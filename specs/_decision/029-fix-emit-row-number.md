# Decisions: fix-emit-row-number

## ADR: Tag every emitted row with the input row it came from

**ID:** emit-echoes-input-row-number
**Plan:** fix-emit-row-number
**Status:** Accepted

### Context

`ExascriptTableData.row_number` ("Local row numbers", field 9 of the vendored `zmqcontainer.proto`) numbers the rows of a batch. The engine numbers every input row it sends and uses the number an emitted row carries to fill the select-list columns the query tunnels through the UDF rather than emitting (`SELECT id, f(x) FROM t`). The reference C++ SLC adds the current input row's number to every emitted row; this runtime sent `row_number: vec![]` and discarded the list on input, so a pass-through query returned wrong values or closed the SQL session. The Tier 2 `scalar_emits_pt` cell reported `incorrect` for exactly this.

One input row may produce many output rows, and one `MT_EMIT` batches rows from many input rows, so the number has to be recorded per buffered row rather than per flush.

### Decision

`InputRowSet` keeps the batch's `row_number` list beside its rows and exposes the current row's number; `EmitBuffer::push` takes that number and `to_proto` emits one entry per row. The Arrow path takes the number once per `push_batch` — every row of a batch comes from the one input row being processed. An input batch that arrives without the list falls back to batch-local indices.

The per-row byte estimate gains a 10-byte term (the widest packed uint64 varint) so it stays an upper bound on the emitted frame and the 4,000,000-byte flush threshold keeps its meaning.

### Options Considered

| Option | Verdict |
|--------|---------|
| Record the number per pushed row | ✓ Chosen — the only shape that survives many output rows per input row and many input rows per flush |
| Hold a "current input row" on the buffer, set by the bridge when the cursor moves | ✗ Rejected — hidden state that goes stale silently; the caller already has the number |
| Number emitted rows sequentially within the flush | ✗ Rejected — the number names an input row, not an output position |

### Consequences

Emitted frames grow by one varint per row (Tier 1 `bytes/row`: native 12.9 → 13.9, strblock 60.6 → 61.6, varchar 56.9 → 57.9, wide 472.2 → 473.2), and the wider estimate flushes marginally earlier. `EmitBuffer::push` and `push_batch` take the row number as an argument, so every caller states which input row it is emitting for. The Tier 1 smoke asserts one `row_number` per emitted row on every cell, and Tier 2 `scalar_emits_pt` now gates the pairing end to end.
