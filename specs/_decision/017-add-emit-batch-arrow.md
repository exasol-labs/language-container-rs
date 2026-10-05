# Decisions: add-emit-batch-arrow

## ADR: Declared EMITS ColumnMeta is authoritative for the target proto block

**ID:** declared-emits-columnmeta-authoritative
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

One Arrow `DataType` maps to several Exasol types. `Utf8` covers VARCHAR, CHAR, GEOMETRY, HASHTYPE and INTERVAL, so the Arrow type alone does not name the proto block.

### Decision

`push_batch` reads each cell by Arrow `DataType` and packs it into the proto block of the declared output `ExaType`, as the row path does.

### Options Considered

| Option | Verdict |
|--------|---------|
| Declared `ColumnMeta` selects the proto block | ✓ Chosen: unambiguous, and byte-identical to the row path |
| Derive the block from the Arrow `DataType` | ✗ Rejected: ambiguous for types that share one Arrow type |

### Consequences

`push_batch` takes the `&[ColumnInfo]` slice. `HostContextBridge` holds `output_meta`.

## ADR: Standalone emit-arrow feature gates only the arrow dependency

**ID:** standalone-emit-arrow-feature
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

Arrow batch emit needs the `arrow` crate. UDFs that do not emit Arrow batches must not pull it in.

### Decision

`exasol-udf-sdk` has the feature `emit-arrow = ["dep:arrow"]`. It gates only the `arrow` dependency and the `RecordBatch` emit extension trait. The connect-back API is not feature-gated. A build without `emit-arrow` compiles no `arrow` dependency.

### Options Considered

| Option | Verdict |
|--------|---------|
| Standalone `emit-arrow` feature | ✓ Chosen: smallest dependency set for UDFs without Arrow |
| Gate `emit_batch` under a connect-back feature | ✗ Rejected: couples two unrelated capabilities |

### Consequences

A UDF that only emits Arrow batches enables `emit-arrow` and gets no tokio or exarrow-rs.
