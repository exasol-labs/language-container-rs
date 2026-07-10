# Decisions: 2026-06-11-vs-adapter-and-single-call-connect-back

## ADR: ABI version bump 2→3 for virtual_schema_adapter_call signature change

**ID:** abi-version-bump-2-3-vs-adapter-call
**Plan:** `2026-06-11-vs-adapter-and-single-call-connect-back`
**Status:** Accepted

### Context

The `virtual_schema_adapter_call` vtable slot changes from a 2-argument signature `(json_arg, result)` to a 3-argument signature `(ctx, json_arg, result)` so VS adapters can receive the host `UdfContext` pointer and call `ctx.connection(...)` / `ctx.connect_back(...)` mid single-call. Any `.so` compiled against ABI v2 that happened to wire this slot would be called with an extra argument, producing undefined behavior. A decision was needed on whether to increment the ABI version or add a parallel slot.

### Decision

Increment `EXA_UDF_ABI_VERSION` from 2 to 3. The `virtual_schema_adapter_call` vtable slot uses the new 3-argument signature exclusively. The loader rejects any `.so` whose `abi_version` field does not equal 3 with a clear version-mismatch error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Increment ABI version 2→3 | ✓ Chosen — slot signature change is a binary incompatibility; incrementing forces the loader to reject old `.so` files with a clear error rather than silently invoking the wrong signature |
| Keep ABI v2, add a separate parallel slot | ✗ Rejected — bloats the vtable and complicates dispatch for no gain; does not eliminate the incompatibility for any `.so` that wired the old slot |
| Struct-based calling convention | ✗ Rejected — adds complexity without addressing the root issue; the double-indirection ABI already proven by the `run` shim is sufficient |

### Consequences

All user `.so` artifacts compiled against ABI v2 must be recompiled. The loader will reject v2 artifacts with a clear version-mismatch error at load time. This dominates the semver bump to 0.5.0 under pre-1.0 rules.

## ADR: Row-major type-block packing with NULL cells skipping the type block

**ID:** row-major-type-block-packing-null-cells
**Plan:** `2026-06-11-vs-adapter-and-single-call-connect-back`
**Status:** Accepted

### Context

The prior `EmitBuffer::to_proto` / `InputRowSet::from_proto` implementation used column-major packing with `n_rows` placeholder entries per column in each type block, including placeholders for NULL cells. This produced silently wrong values when NULL cells appeared: a NULL in column 2 would still write a placeholder into the string block, causing all subsequent string values for that row to land in the wrong column position. The Exasol wire format is row-major with no NULL slots.

### Decision

Switch `EmitBuffer::to_proto` and `InputRowSet::from_proto` to row-major ordering within each type block (row then column). NULL cells do not push any placeholder slot into the type block — only the null-bitmap is updated. Per-type running cursors advance only on non-null cells. Output values are packed by declared column `ExaType`, not by runtime `Value` variant (e.g. a `Value::Int64` in a `Numeric` column is stringified into the string block).

### Options Considered

| Option | Verdict |
|--------|---------|
| Row-major packing, NULL cells skip type-block slot | ✓ Chosen — matches the confirmed C++ reference behavior; eliminates the silent column-value corruption; per-type cursors correctly handle mixed-NULL rows |
| Column-major with `n_rows` placeholder entries per column | ✗ Rejected — produced silently wrong values when NULLs appeared; confirmed to be incorrect |
| Row-major with NULL placeholders | ✗ Rejected — still produces wrong values; the placeholder is the root cause |

### Consequences

All output type blocks are now row-major. NULL cells occupy no slot in their type block — only the null-bitmap is set. The `push_placeholder` function is removed as dead code. This is a correctness fix; the wire format now matches the Exasol reference implementation.
