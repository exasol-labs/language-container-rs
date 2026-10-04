# Decisions: 2026-06-11-vs-adapter-and-single-call-connect-back

## ADR: ABI version bump 2→3 for virtual_schema_adapter_call signature change

**ID:** abi-version-bump-2-3-vs-adapter-call
**Plan:** `2026-06-11-vs-adapter-and-single-call-connect-back`
**Status:** Accepted

### Context

The `virtual_schema_adapter_call` vtable slot takes `(ctx, json_arg, result)` so VS adapters can call `ctx.connection(...)` and `ctx.connect_back(...)` during a single call. A `.so` built against the 2-argument slot would be called with an extra argument, which is undefined behavior.

### Decision

`EXA_UDF_ABI_VERSION` is 3. The loader rejects any `.so` whose `abi_version` is not 3 with a version-mismatch error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Increment the ABI version | ✓ Chosen, rejects old `.so` files with a clear error |
| Keep the version, add a parallel slot | ✗ Bloats the vtable, does not remove the incompatibility |
| Struct-based calling convention | ✗ Extra complexity, the `run` shim's double indirection suffices |

### Consequences

User `.so` artifacts built against an older ABI must be recompiled.

## ADR: Row-major type-block packing with NULL cells skipping the type block

**ID:** row-major-type-block-packing-null-cells
**Plan:** `2026-06-11-vs-adapter-and-single-call-connect-back`
**Status:** Accepted

### Context

The Exasol wire format is row-major with no NULL slots. A placeholder entry for a NULL cell shifts every later value in its type block into the wrong column.

### Decision

`EmitBuffer::to_proto` and `InputRowSet::from_proto` order each type block row-major (row, then column). A NULL cell sets only the null-bitmap and adds no slot to the type block. Per-type cursors advance only on non-null cells. Output values are packed by declared column `ExaType`, not by runtime `Value` variant (a `Value::Int64` in a `Numeric` column is stringified into the string block).

### Options Considered

| Option | Verdict |
|--------|---------|
| Row-major, NULL cells skip the slot | ✓ Chosen, matches the C++ reference |
| Column-major with `n_rows` placeholders per column | ✗ Corrupts values after a NULL |
| Row-major with NULL placeholders | ✗ The placeholder causes the corruption |

### Consequences

None beyond the decision.
