# Decisions: fix-emit-validation-column-metadata

## ADR: Emitted rows are validated in the bridge, before they reach the buffer

**ID:** emit-validates-against-output-columns
**Plan:** `fix-emit-validation-column-metadata`
**Status:** Accepted

### Context

`EmitBuffer::take_proto` packs each row by iterating the declared output columns, not
the emitted row. A short row's missing cells became NULL, a long row's extras were
dropped, and every cell was coerced by column type: a `String` into an integer column
parsed to `0`, a `Value::Int64` past `i32::MAX` wrapped into an INT32 column, and any
variant was stringified into a NUMERIC/DATE/TIMESTAMP block, `NaN` and `1e21` included.

Nothing downstream catches this. The database acknowledges `MT_EMIT` before it reads
the rows and reports no per-row problem, and `schema_check` runs once at handshake on
the vtable's declared shape rather than per row. Every mismatch surfaced as a wrong
query result.

### Decision

`HostContextBridge::push_output_row` checks arity and per-cell type against the
declared output columns before the row is buffered, so the error names the offending
column and travels the existing `UdfError` → `MT_CLOSE` path to the SQL user. It sits
on the shared push, so `emit` (EMITS) and `set_return` (RETURNS) obey one contract.

Acceptance is by declared column, not by strict variant equality: Exasol delivers
`BIGINT` as `PB_NUMERIC`, so `Int32`/`Int64` into a NUMERIC column is the common case
rather than an error, and `Int64` into an INT32 column is range-checked instead of
refused. `Double` into NUMERIC is rejected: its `Display` form is not always a DECIMAL
literal. `Null` is valid in every column.

### Options Considered

| Option | Verdict |
|--------|---------|
| Check in the bridge, before `push` | ✓ Chosen — the only point that still knows which `emit` call produced the row |
| Check in `take_proto` | ✗ Rejected — one flush batches many rows from many calls, so the error can name no call site |
| Make the `value_to_*` coercions lossless instead | ✗ Rejected — a lossless coercion of a `String` into an integer column is still a wrong result, silently |
| Leave it to the database | ✗ Rejected — the emit ack precedes the read; no per-row error ever comes back |

### Consequences

A UDF that relied on a coercion now fails the query instead of returning a wrong value.

The check is fused into the byte-cost pass `push` already made — it returns the row cost
and the buffer takes it through `push_costed` — so an emitted row is still walked once.
As a separate pass it cost a few percent on the row cells, scaling with columns per row
(`wide_row`, 24 columns, was the clearest); fused, Tier 1 puts seven of the eight row
cells inside the noise band over three interleaved pairs, with `scalar_emits_gen/wide_row`
at −3.0 %. Unchanged code swings ±10 % run to run on this harness.

## ADR: One owned column-metadata type, owned by the SDK

**ID:** sdk-owns-column-info
**Plan:** `fix-emit-validation-column-metadata`
**Status:** Accepted

### Context

A UDF saw nothing of its own schema but the input column count. One whose output shape
comes from the call-site `EMITS` list had to be handed a redundant column plan as a
parameter, and reordering the `EMITS` list then wrote values into the wrong columns.
The data was already in the bridge as the protocol crate's `ColumnMeta`, which the SDK
cannot name.

### Decision

`ColumnMeta` moves into the SDK as `ColumnInfo`, and `exa-zmq-protocol` re-exports it
the way it already re-exports `ExaType`. `UdfContext::input_column` and `output_column`
return `&ColumnInfo` out of the slices the bridge already holds, so the accessors
allocate nothing and no second borrowed-view type exists.

`ColumnMeta::from_pb` becomes the free function `column_from_pb`, since an inherent
`impl` on a type from another crate is not available.

### Options Considered

| Option | Verdict |
|--------|---------|
| Move the owned type into the SDK, return `&ColumnInfo` | ✓ Chosen — one type for one concept, and no allocation per access |
| Add a borrowed `ColumnInfo<'a>` view beside `ColumnMeta` | ✗ Rejected — two types, six duplicated fields, and the test double still needs owned storage |
| Return owned `String` fields per call | ✗ Rejected — allocates on every access to describe a schema that never changes |

### Consequences

The `dyn UdfContext` vtable gains three methods, so `EXA_UDF_ABI_VERSION` is bumped
8 → 9 and every downstream UDF rebuilds. The three methods are defaulted, so an
existing `impl UdfContext` keeps compiling.
