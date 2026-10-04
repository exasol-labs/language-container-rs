# Decisions: fix-abi-feature-safety

## ADR: Remove `query_arrow`; no Arrow accessor replacement

**ID:** remove-query-arrow-no-replacement
**Plan:** `fix-abi-feature-safety`
**Status:** Accepted

### Context

A UDF `.so` and the host each link their own static `arrow`, so `Vec<arrow::RecordBatch>` crossing the boundary yields silently wrong `TypeId`/vtable comparisons. Arrow IPC ser/deser is only 2-9% of `emit_batch` cost, so an Arrow path gains no throughput over `Vec<Value>`.

### Decision

`ExaConnection` has no `query_arrow`. `query_for_each` (a `Vec<Value>` row callback) is the required streaming method, and `query` defaults to collecting it. No `query_arrow_ffi` or Arrow C Data Interface replacement exists.

### Options Considered

| Option | Verdict |
|--------|---------|
| Remove `query_arrow`, no replacement | ✓ Chosen |
| `#[deprecated]` but keep | ✗ Unsafe API still compiles |
| Replace with `query_arrow_ffi` | ✗ No throughput gain |
| Gate behind a feature | ✗ Hazard remains in feature-enabled builds |

### Consequences

`ExaConnection` is arrow-free, so the `connect_back` module compiles without an optional `arrow` dependency.

## ADR: Make the `UdfContext` trait-object vtable feature-independent

**ID:** udfcontext-vtable-feature-independent
**Plan:** `fix-abi-feature-safety`
**Status:** Accepted

### Context

The `&mut dyn UdfContext` vtable is ordered by method declaration. Feature-gated methods give a `.so` and the host different layouts, so `ctx.emit_batch()` can dispatch to the wrong method and emit 0 rows without error. The ABI fingerprint does not encode feature flags.

### Decision

`UdfContext` declares `cluster_ip`, `connection`, `connect_back` and `emit_record_batch_ipc` unconditionally, with `Unimplemented` defaults. No `UdfContext` method carries a `#[cfg(feature = ...)]`. The `emit-arrow` feature gates only `dep:arrow` and the `EmitBatch` extension trait. `EXA_UDF_ABI_VERSION` is 5, so a `.so` with the old layout fails with `AbiMismatch`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Unconditional declarations with `Unimplemented` defaults | ✓ Chosen: one layout in all builds |
| Encode the feature set in the ABI fingerprint | ✗ Detects the skew but cannot interoperate |
| Separate `#[repr(C)]` context vtable | ✗ Heavy; leaves the root cause |

### Consequences

Each `UdfContext` method resolves to the same vtable slot under any feature set.
