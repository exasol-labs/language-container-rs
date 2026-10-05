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

## ADR: Standalone emit-arrow feature; connect-back implies it

**ID:** standalone-emit-arrow-feature
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

Arrow batch emit needs the `arrow` crate. Pure-compute UDFs must not pull in tokio, exarrow-rs and rustls to use it.

### Decision

`exasol-udf-sdk` has the feature `emit-arrow = ["dep:arrow"]`, and `connect-back` implies it. A build with neither feature compiles no `arrow` dependency.

### Options Considered

| Option | Verdict |
|--------|---------|
| Standalone `emit-arrow` feature that `connect-back` implies | ✓ Chosen: smallest dependency set for pure-compute UDFs |
| Gate `emit_batch` under `connect-back` | ✗ Rejected: forces tokio, exarrow-rs and rustls on pure-compute UDFs |

### Consequences

A UDF that only emits Arrow batches enables `emit-arrow` and gets no tokio or exarrow-rs.
