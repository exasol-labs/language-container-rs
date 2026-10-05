# Decisions: fix-emit-validation-column-metadata

## ADR: Emitted rows are validated in the bridge, before they reach the buffer

**ID:** emit-validates-against-output-columns
**Plan:** `fix-emit-validation-column-metadata`
**Status:** Accepted

### Context

The emit buffer packs rows by the declared output columns, so a short row gains NULLs, a long row loses cells, and cells are coerced by column type. The DB acknowledges `MT_EMIT` before reading rows, and `schema_check` runs once at handshake, so every mismatch becomes a wrong query result.

### Decision

`HostContextBridge::push_output_row` checks arity and per-cell type against the declared output columns before buffering. The error names the offending column and travels the `UdfError` → `MT_CLOSE` path to the SQL user. The check sits on the shared push, so `emit` (EMITS) and `set_return` (RETURNS) obey one contract.

Acceptance is by declared column, not by strict variant equality. `Int32`/`Int64` into a NUMERIC column is accepted because Exasol delivers `BIGINT` as `PB_NUMERIC`. `Int64` into an INT32 column is range-checked. `Double` into NUMERIC is rejected because its `Display` form is not always a DECIMAL literal. `Null` is valid in every column.

### Options Considered

| Option | Verdict |
|--------|---------|
| Check in the bridge, before `push` | ✓ Chosen |
| Check at flush | ✗ One flush batches rows from many calls, so the error names no call site |
| Make the `value_to_*` coercions lossless | ✗ A lossless coercion of a `String` into an integer column is still a wrong result |
| Leave it to the database | ✗ No per-row error comes back |

### Consequences

A UDF that relied on a coercion fails the query. Validation shares the byte-cost pass `push` already makes and adds no separate row traversal.

## ADR: One column-metadata type, owned by the SDK

**ID:** sdk-owns-column-info
**Plan:** `fix-emit-validation-column-metadata`
**Status:** Accepted

### Context

A UDF needs its own schema beyond the input column count, and the SDK cannot name a type that lives in the protocol crate. A UDF whose output shape comes from the call-site `EMITS` list otherwise needs a redundant column plan.

### Decision

`ColumnInfo` is the SDK's column-metadata type, and `exa-zmq-protocol` re-exports it as it does `ExaType`. `UdfContext::input_column` and `output_column` return `&ColumnInfo` from the slices the bridge holds, so the accessors allocate nothing. `column_from_pb` is a free function, because an inherent `impl` on a foreign type is unavailable.

### Options Considered

| Option | Verdict |
|--------|---------|
| Owned type in the SDK, return `&ColumnInfo` | ✓ Chosen |
| Borrowed `ColumnInfo<'a>` view beside a protocol-crate type | ✗ Two types with duplicated fields, and the test double still needs owned storage |
| Return owned `String` fields per call | ✗ Allocates on every access to describe a fixed schema |

### Consequences

The three column accessors are defaulted `dyn UdfContext` vtable methods, so an existing `impl UdfContext` keeps compiling. Adding them changes the vtable, so `EXA_UDF_ABI_VERSION` is bumped and downstream UDFs rebuild.
