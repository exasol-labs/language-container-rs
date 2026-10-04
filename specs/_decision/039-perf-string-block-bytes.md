# Decisions: perf-string-block-bytes

## ADR: The string block travels as `bytes` and `MT_EMIT` is hand-encoded

**ID:** string-block-bytes-hand-encoded-emit
**Plan:** `perf-string-block-bytes`
**Status:** Accepted

### Context

`repeated string` allocates one `String` per cell during prost decode and encode. Slicing an arena into `Vec<Bytes>` costs as much per cell as the `String` it replaces.

### Decision

`data_string` is declared `repeated bytes`, which has the same wire encoding. Received frames are wrapped as `Bytes` without a copy, so decoded cells slice the frame. Each cell is validated as UTF-8 once, when converted to a `Value`, and a non-UTF-8 cell fails the batch.

On emit, `EmitTable` keeps the string block in wire form (key, length and bytes per cell) in one buffer that the NUMERIC, DATE and TIMESTAMP formatters write into directly. `EmitRequest` writes the `MT_EMIT` frame field by field in prost's tag order with `prost::encoding`, copying the string block in one piece. The frame is byte-identical to prost's encoding of the same `ExascriptRequest`. The buffer keeps its blocks across flushes.

### Options Considered

| Option | Verdict |
|--------|---------|
| `bytes` on the wire, hand-encoded `MT_EMIT` from a wire-form block | ✓ Chosen |
| `bytes` plus `Vec<Bytes>` sliced from an arena | ✗ Slower on string-block emit cells |
| Keep `repeated string` | ✗ One allocation per string cell in each direction |

### Consequences

The vendored `zmqcontainer.proto` differs from upstream in this one field type. The hand encoder must follow `exascript_request`, `exascript_emit_data_req` and `exascript_table_data`, and a test checks random tables byte-for-byte against prost. `EmitBuffer::take_proto` remains only as the `ExascriptTableData` view for tests.
