# Decisions: perf-string-block-bytes

## ADR: The string block travels as `bytes` and `MT_EMIT` is hand-encoded

**ID:** string-block-bytes-hand-encoded-emit
**Plan:** `perf-string-block-bytes`
**Status:** Accepted

### Context

`string-block-stays-repeated-string` (034) kept `repeated string` because slicing one arena into
`Vec<Bytes>` cost as much per cell as the `String` it replaced, and rejected a hand-written
encoder as a second protobuf encoder to keep byte-identical. On ingest, `repeated string` also
allocates one `String` per cell during prost decode, the largest single cost of wide input rows.

### Decision

`data_string` is declared `repeated bytes`, which is the same wire encoding. Received frames
are wrapped as `Bytes` without a copy, so decoded cells slice the frame, and each cell is
validated as UTF-8 once, when it is converted to a `Value`. A non-UTF-8 cell fails the batch,
as prost's `string` check did.

The emit side has no per-cell handle. `EmitTable` keeps the string block already in wire form,
key, length and bytes per cell, in one buffer that the NUMERIC, DATE and TIMESTAMP formatters
write into directly. `EmitRequest` writes the `MT_EMIT` frame field by field in prost's tag
order with `prost::encoding`, copying the string block in one piece, so the frame is
byte-identical to prost's encoding of the same `ExascriptRequest`. The buffer keeps its blocks
across flushes.

### Options Considered

| Option | Verdict |
|--------|---------|
| `bytes` on the wire plus a hand-encoded `MT_EMIT` from a wire-form block | ✓ Chosen: removes the per-cell allocation on both sides |
| `bytes` plus `Vec<Bytes>` sliced from an arena (034's experiment) | ✗ Rejected: +13…+35 % on string-block emit cells in 034 |
| Keep `repeated string` | ✗ Rejected: leaves one allocation per string cell in each direction |

### Consequences

The vendored `zmqcontainer.proto` differs from upstream in the one field type. The hand encoder
must follow `exascript_request`, `exascript_emit_data_req` and `exascript_table_data`. A test
checks random tables byte-for-byte against prost. `EmitBuffer::take_proto` remains only as the
`ExascriptTableData` view for tests.

Tier 1 `full` against `main`: `set_returns` wide_g1 −33 %, strblock_g1 −22 %;
`scalar_emits_gen` strblock −39…−45 %, wide −31…−38 %, varchar −4…−7 %; `set_emits` strblock
−25…−36 %, wide −13…−34 %. Native cells are unchanged, and so are the `MT_EMIT` counters.

Tier 2 `full`, three alternating runs per side on docker-db 2026.1.1 against `main`:
`scalar_emits_gen` wide_batch8k −22 %, wide_batch64k −16 %, wide_row −12 %, strblock_row −12 %;
`set_gen` wide_batch8k −15 %, strblock_batch −12 %. No cell regressed; `varchar_row` +5 % and
`set_returns_native_g1` +5 % are inside the band. The strblock and wide input cells are bound by
the engine and unchanged.
