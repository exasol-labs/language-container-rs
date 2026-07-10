# Decisions: fix-abi-feature-safety

## ADR: Remove `query_arrow`; no Arrow accessor replacement (#26)

**ID:** remove-query-arrow-no-replacement
**Plan:** `fix-abi-feature-safety`
**Status:** Accepted

### Context

`ExaConnection::query_arrow` returned `Vec<arrow::RecordBatch>` across the `.so` boundary. A UDF `.so` and the host each link their own static `arrow`, so `TypeId`/vtable comparisons on those batches silently return wrong values — no error, no panic, just corrupted data (issue #26). The Arrow IPC emit-throughput benchmark also showed Arrow IPC ser/deser is only 2–9% of `emit_batch`'s cost, so an Arrow streaming path buys nothing over `Vec<Value>`.

### Decision

Drop `ExaConnection::query_arrow`. Make `query_for_each` (`Vec<Value>` row callback) the required streaming method; `query` defaults to collecting it. Do not add a `query_arrow_ffi` or Arrow C Data Interface replacement — `Vec<Value>` is already safe and ergonomic.

### Options Considered

| Option | Verdict |
|--------|---------|
| Remove `query_arrow`, no replacement | ✓ Chosen — eliminates the footgun; `Vec<Value>` is sufficient and safe |
| `#[deprecated]` but keep | ✗ Rejected — deprecated unsafe API still compiles; silent UB still reachable |
| Replace with `query_arrow_ffi` (Arrow C Data Interface) | ✗ Rejected — Arrow C Data Interface scope dropped; benchmark showed no throughput gain |
| Gate behind a feature | ✗ Rejected — vestigial gated unsafe API re-introduces the hazard for feature-enabled builds |

### Consequences

`ExaConnection` becomes arrow-free. All connect-back results are delivered as `Vec<Value>` rows — the same type that crosses every other SDK boundary. This is also the prerequisite for making `UdfContext` feature-independent (see ADR-052): without `query_arrow`, the `connect_back` module needs no optional `arrow` dependency, so it can compile unconditionally.

## ADR: Make the `UdfContext` trait-object vtable feature-independent (#31)

**ID:** udfcontext-vtable-feature-independent
**Plan:** `fix-abi-feature-safety`
**Status:** Accepted

### Context

The `.so`↔host call context crosses as a `&mut dyn UdfContext` Rust trait object inside the run shim. That trait-object vtable is ordered by method declaration. Several `UdfContext` methods were `#[cfg(feature = ...)]`-gated: `cluster_ip`/`connection`/`connect_back` behind `connect-back`, and `emit_record_batch_ipc` behind `emit-arrow`. A UDF `.so` built without `connect-back` but with `emit-arrow` places `emit_record_batch_ipc` in an earlier vtable slot than the host (which was built with both features). `ctx.emit_batch()` therefore silently dispatched to `cluster_ip`, returned `Ok`, and emitted 0 rows with no error (issue #31). The ABI fingerprint did not encode feature flags, so the loader did not catch it.

### Decision

Remove every `#[cfg(feature = ...)]` from `UdfContext` method declarations. Declare `cluster_ip`, `connection`, `connect_back`, and `emit_record_batch_ipc` unconditionally with `Unimplemented` defaults so the `dyn UdfContext` vtable layout is identical in all feature configurations. Narrow the `emit-arrow` feature to gate only `dep:arrow` and the `EmitBatch` extension trait; it no longer gates any trait method declaration. Bump `EXA_UDF_ABI_VERSION` 4 → 5 so a `.so` compiled against the old layout fails the loader's version check with a clear `AbiMismatch` error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Unconditional method declarations with `Unimplemented` defaults | ✓ Chosen — eliminates the vtable-skew class entirely; all builds get the same layout |
| Encode feature set in the ABI fingerprint (detect-only) | ✗ Rejected — detect-only still cannot interoperate; structural fix is strictly better |
| Separate `#[repr(C)]` context vtable | ✗ Rejected — heavy; shifts complexity without removing the root cause |

### Consequences

Every `UdfContext` method resolves to the same vtable slot regardless of which cargo features a UDF enables. The `emit-arrow`-only UDF emit-batch dispatch bug (#31) is eliminated at the structural level. Fingerprint-feature encoding remains a possible defense-in-depth follow-up but is not required once the layout is stable. Old `.so` artifacts built against ABI v4 are rejected loudly rather than misdispatching.
