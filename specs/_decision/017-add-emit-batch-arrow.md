# Decisions: add-emit-batch-arrow

## ADR: Declared EMITS ColumnMeta (not Arrow schema) is authoritative for the target proto block

**ID:** declared-emits-columnmeta-authoritative
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

`EmitBuffer::push_batch` must decide which proto type block (double_data, int32_data, string_data, …) to pack each Arrow column's values into. Two approaches were considered: derive the target block from the Arrow `DataType` alone, or use the declared EMITS output `ColumnMeta` (the same `ExaType` the row-based `to_proto` path already uses).

### Decision

`push_batch` dispatches on the Arrow `DataType` only to extract each cell value, then packs it into the proto block dictated by the declared output `ExaType` — identical to the existing `to_proto` row path. The declared EMITS schema is the single source of truth for the target proto block.

### Options Considered

| Option | Verdict |
|--------|---------|
| Declared EMITS `ColumnMeta` (`ExaType`) dictates the target proto block | ✓ Chosen — Arrow `DataType` is ambiguous for Exasol: `Utf8` maps to VARCHAR/CHAR/GEOMETRY/HASHTYPE/INTERVAL; only the declared schema names the correct block; also keeps `push_batch` byte-identical to the row path |
| Derive target block purely from Arrow `DataType` (no `ColumnMeta`) | ✗ Rejected — ambiguous: multiple Exasol types collapse onto one Arrow type; produces wrong block for extended Exasol types without consulting the declared schema |

### Consequences

`push_batch` requires the `&[ColumnMeta]` slice at call time (the same slice the flusher serialises with), making the output schema an explicit dependency of the Arrow batch-emit path. The `HostContextBridge` carries `output_meta` in its struct fields, threaded in by dispatch at construction.

## ADR: Standalone emit-arrow feature; connect-back implies it

**ID:** standalone-emit-arrow-feature
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

Arrow batch emit (`emit_batch`) requires the `arrow` crate. The question was whether to gate it under the existing `connect-back` feature or add a new independent feature so pure-compute UDFs (no connect-back) can emit Arrow batches without pulling in tokio, exarrow-rs, and rustls.

### Decision

Add `emit-arrow = ["dep:arrow"]` to `exasol-udf-sdk`; add `emit-arrow` to the `connect-back` feature list so connect-back continues to pull in arrow transitively. Building with neither feature compiles no `arrow` dependency.

### Options Considered

| Option | Verdict |
|--------|---------|
| New standalone `emit-arrow` feature; `connect-back` implies it | ✓ Chosen — minimal dependency surface for pure-compute UDFs; connect-back UDFs get it for free; mirrors the SDK's existing pattern of additive, independent feature flags |
| Gate `emit_batch` directly under `connect-back` | ✗ Rejected — forces pure-compute UDFs to pull in tokio/exarrow-rs/rustls to emit Arrow batches; violates the principle of minimal dependency surface |

### Consequences

UDF crates that want only Arrow batch emit add `emit-arrow` to their `exasol-udf-sdk` dependency and get no transitive tokio or exarrow-rs. Connect-back UDFs gain `emit_batch` automatically. The `arrow` dependency is optional (no implicit compile cost for `emit`-only UDFs).

## ADR: Vectorised column-at-a-time Arrow encoding with row-granular 4 MB split points

**ID:** vectorised-column-at-a-time-arrow-encoding
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

`EmitBuffer::push_batch` must encode an Arrow `RecordBatch` into proto type blocks without a `Vec<Value>` intermediate, while respecting the hard 4 MB `MT_EMIT` wire limit. A batch's serialised size is unknown at compile time and can exceed 4 MB, so flushing strictly at batch boundaries is incorrect. The proto type blocks use a dense, row-major-interleaved layout (non-null cells only) that must be preserved for `from_proto` to decode correctly.

### Decision

Encode column-at-a-time (vectorised): downcast each Arrow column array once and read its null buffer once in bulk, then assemble values row-by-row into the interleaved proto blocks. A cheap cumulative per-row byte cost (fixed-width by width; variable-width from the Arrow offset buffer; Decimal/Timestamp via the same fixed estimate as `value_byte_cost`) is computed without touching data bytes. Row-granular split points at the 4 MB threshold yield zero-copy `RecordBatch::slice` segments that are encoded and flushed; the sub-4 MB trailing remainder is materialised once into the shared `Vec<Value>` buffer so the row path and the end-of-run tail flush remain coherent.

### Options Considered

| Option | Verdict |
|--------|---------|
| Vectorised column-at-a-time downcast + row-major block assembly + row-granular 4 MB splits | ✓ Chosen — eliminates per-cell downcast and Value allocation for the bulk of the data; offset-buffer prefix-sum gives split points without touching data bytes; correct for all EMITS schemas including multi-column same-ExaType and nulls |
| Flush strictly at batch boundaries | ✗ Rejected — a batch can exceed the hard 4 MB wire cap; incorrect, not merely slow |
| Cell-by-cell extraction (downcast per cell, per-cell Value allocation) | ✗ Rejected — the per-row pivot the feature exists to remove; equivalent to the existing `record_batch_to_rows` path |
| True bulk `extend` of whole columns into blocks | ✗ Rejected — scrambles the row-major-interleaved block layout whenever multiple columns share a block type or nulls are present; not correct for arbitrary EMITS schemas |

### Consequences

`push_batch` is faster than the row pivot for large batches (one downcast per column, not per cell; no intermediate `Value` vec for bulk data). The trailing <4 MB remainder is the only `Vec<Value>` materialisation on the batch path — bounded by the same 4 MB the row path already buffers. The row `emit`/`to_proto` path and all flush semantics are unchanged, keeping regression surface minimal.
