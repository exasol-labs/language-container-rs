# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext`/`UdfRun` traits, the `Value`/`ExaType` model, and the ABI vtable. This delta makes the `UdfContext` trait-object vtable **feature-independent** to close the silent emit/dispatch corruption of issue #31, and bumps the ABI version so any stale `.so` is rejected loudly instead of misdispatching.

## Background

The `.so`↔host boundary is a stable `#[repr(C)] ExaUdfVTable` (abi.rs), but the call context crosses as a `&mut dyn UdfContext` Rust trait object inside the run shim. That trait-object vtable is ordered by method declaration, and several `UdfContext` methods were `#[cfg(feature = ...)]`-gated: `cluster_ip`/`connection`/`connect_back` behind `connect-back`, and `emit_record_batch_ipc` behind `emit-arrow` (declared last).

A UDF `.so` built with a different feature set than the host SLC therefore gets a **different vtable layout**. The host (built with `connect-back` + `emit-arrow`) places `emit_record_batch_ipc` after the connect-back methods; a UDF built with `emit-arrow` only places it earlier — onto the host's `cluster_ip` slot. `ctx.emit_batch()` then silently calls `cluster_ip`, returns `Ok`, and emits **0 rows with no error** (issue #31). The ABI fingerprint (`SDK_VERSION:RUSTC_HASH`) does not encode features, so the loader does not catch it.

Fix: declare every `UdfContext` method **unconditionally** (with `Unimplemented` defaults), so the trait-object vtable layout is identical regardless of which cargo features a UDF enables. The connect-back types this requires now compile unconditionally (see `sdk/connect-back`). The `emit-arrow` feature is narrowed to gate only the optional `arrow` dependency and the `RecordBatch`-taking `EmitBatch` ext-trait — not the `&[u8]`-only `emit_record_batch_ipc` ABI method. The ABI version is bumped so a `.so` compiled against the old layout fails the loader's version check cleanly.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: UdfContext vtable layout is feature-independent

* *GIVEN* the `exasol-udf-sdk` crate built with any combination of cargo features
* *WHEN* the `UdfContext` trait is compiled
* *THEN* every method — including `cluster_ip`, `connection`, `connect_back`, and `emit_record_batch_ipc` — MUST be declared **unconditionally** (no `#[cfg(feature = ...)]` on any trait method), each with a default implementation returning `UdfError::Unimplemented`, so the method set and declaration order (hence the `dyn UdfContext` vtable layout) are identical in every build configuration
* *AND* `connection` and `connect_back` MUST reference `ConnectionObject` / `ExaConnection` from the now-unconditionally-compiled `connect_back` module (see `sdk/connect-back`), so their signatures compile without any feature gate
* *AND* `emit_record_batch_ipc(&mut self, ipc: &[u8]) -> Result<(), UdfError>` MUST take only `&[u8]` (no `arrow` type), so it can be declared unconditionally even when the `arrow` dependency is absent
* *AND* a UDF `.so` and the host built with different feature sets MUST resolve every `UdfContext` method to the same vtable slot, so `ctx.emit_batch()` from an `emit-arrow`-only UDF dispatches to the host's `emit_record_batch_ipc` (not `cluster_ip`) and emits all rows (issue #31)
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: emit-arrow gates only the arrow dependency and the RecordBatch ext-trait

* *GIVEN* the SDK compiled without the `emit-arrow` feature
* *WHEN* the crate is built
* *THEN* the optional `arrow` dependency MUST NOT be compiled and the `EmitBatch` extension trait (`emit_batch(&RecordBatch)`, which serialises to Arrow IPC bytes in the caller crate) MUST NOT be present, because it is the only API that names an `arrow` type
* *AND* the `UdfContext::emit_record_batch_ipc(&[u8])` trait method MUST still be present (it names no `arrow` type), so the vtable is unchanged whether or not `emit-arrow` is enabled
* *AND* with `emit-arrow` enabled, `emit_batch` MUST serialise the `RecordBatch` to Arrow IPC bytes and forward them to `emit_record_batch_ipc(&[u8])`; an Arrow `RecordBatch` MUST NOT cross the `.so` boundary, only `&[u8]`
* *AND* the row-based `emit(&mut self, values: &[Value])` MUST remain a required trait method, so a UDF MAY mix `emit` and `emit_batch` within one `run`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: ABI version is bumped so a stale .so is rejected, not misdispatched

* *GIVEN* the loader compares a loaded `.so`'s `ExaUdfVTable.abi_version` against the host's `EXA_UDF_ABI_VERSION`
* *WHEN* the `UdfContext` trait layout changes under this delta (feature-independent vtable)
* *THEN* `EXA_UDF_ABI_VERSION` MUST be incremented (4 → 5), so a `.so` compiled against the previous, feature-dependent layout fails the loader's existing version check with a clear `AbiMismatch` error instead of silently calling the wrong vtable slot
* *AND* the `#[repr(C)] ExaUdfVTable` struct field order MUST remain unchanged, so the version check itself remains readable across the boundary (the bump signals the `dyn UdfContext` layout change, which the struct does not otherwise encode)
<!-- /DELTA:CHANGED -->
