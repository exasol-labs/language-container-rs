# Decisions: fix-emit-row-number

## ADR: Tag every emitted row with the input row it came from

**ID:** emit-echoes-input-row-number
**Plan:** fix-emit-row-number
**Status:** Accepted

### Context

The engine uses the `row_number` an emitted row carries (`ExascriptTableData.row_number`, field 9 of `zmqcontainer.proto`) to fill select-list columns tunnelled through the UDF, as in `SELECT id, f(x) FROM t`. One input row can produce many output rows, and one `MT_EMIT` can batch rows from many input rows, so the number is recorded per buffered row.

### Decision

`InputRowSet` keeps the batch's `row_number` list beside its rows and exposes the current row's number. `EmitBuffer::push` takes that number, and the encoded frame carries one entry per row. The Arrow path takes the number once per `push_batch`. An input batch without the list falls back to batch-local indices. The per-row byte estimate includes 10 bytes for the widest packed uint64 varint, so it stays an upper bound on the frame and the 4,000,000-byte flush threshold keeps its meaning.

### Options Considered

| Option | Verdict |
|--------|---------|
| Record the number per pushed row | ✓ Chosen |
| Hold a "current input row" on the buffer | ✗ Hidden state that goes stale silently |
| Number emitted rows sequentially within the flush | ✗ The number names an input row, not an output position |

### Consequences

Emitted frames carry one extra varint per row. `EmitBuffer::push` and `push_batch` take the row number as an argument, so every caller states which input row it emits for.
