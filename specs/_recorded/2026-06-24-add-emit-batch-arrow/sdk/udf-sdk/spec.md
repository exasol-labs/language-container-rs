# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext`/`UdfRun` traits, the `Value`/`ExaType` model, and the ABI vtable. This delta adds an opt-in Arrow batch emit path behind a new `emit-arrow` feature.

## Background

This delta adds a new standalone `emit-arrow` feature to `exasol-udf-sdk` that activates the optional `arrow` dependency independently of `connect-back`, and the `connect-back` feature implies `emit-arrow`. Behind that feature, authors MAY emit a whole Arrow `RecordBatch` alongside the existing row-based `emit` by calling `emit_batch(&RecordBatch)`, exposed through a blanket `EmitBatch` extension trait. Crucially, Arrow types MUST NOT cross the `.so` boundary (two statically linked `arrow` copies disagree on `Arc<dyn Array>` vtables and `TypeId`, which SIGSEGVs — the same hazard as connect-back `query_arrow`): the `EmitBatch::emit_batch` blanket impl serialises the batch to Arrow IPC bytes **in the caller (UDF) crate** and crosses the boundary only as `&[u8]` through the defaulted `UdfContext::emit_record_batch_ipc(&[u8])` ABI method, whose default returns `UdfError::Unimplemented` so existing context impls keep compiling.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: emit-arrow feature pulls in arrow independently of connect-back

* *GIVEN* the `exasol-udf-sdk` crate's `Cargo.toml`
* *WHEN* its feature table is read
* *THEN* it MUST declare an `emit-arrow` feature that activates the optional `arrow` dependency (`dep:arrow`), so a UDF crate can emit Arrow batches without enabling `connect-back`
* *AND* the `connect-back` feature MUST imply `emit-arrow` (its feature list MUST include `emit-arrow`), because connect-back UDFs are the primary consumers of Arrow batch emit and already depend on `arrow`
* *AND* building the crate with neither feature MUST NOT compile in the `arrow` dependency, preserving the lean default for UDFs that use only the row-based `emit`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: emit_batch serialises to Arrow IPC bytes so Arrow never crosses the .so boundary

* *GIVEN* the SDK compiled with the `emit-arrow` feature enabled
* *WHEN* a UDF author calls `ctx.emit_batch(&batch)` on a `&mut dyn UdfContext`
* *THEN* the SDK MUST expose `emit_batch(&mut self, batch: &arrow::record_batch::RecordBatch) -> Result<(), UdfError>` via a public blanket extension trait (`impl<C: UdfContext + ?Sized> EmitBatch for C`) gated `#[cfg(feature = "emit-arrow")]`, so the serialisation is monomorphised in the caller (UDF) crate and runs against the UDF's own `arrow`
* *AND* `emit_batch` MUST serialise the `RecordBatch` to Arrow IPC bytes and forward them to `self.emit_record_batch_ipc(&[u8])` — an Arrow `RecordBatch` MUST NOT be passed across the `.so` boundary, because two independently linked static `arrow` copies disagree on `Arc<dyn Array>` vtables and `TypeId` (a hard memory fault, the same hazard documented for connect-back `query_arrow`); only `&[u8]` crosses
* *AND* `UdfContext` MUST expose a defaulted `emit_record_batch_ipc(&mut self, ipc: &[u8]) -> Result<(), UdfError>` gated `#[cfg(feature = "emit-arrow")]` whose default returns `Err(UdfError::Unimplemented("emit_record_batch_ipc"))`, so existing `UdfContext` implementations that do not override it keep compiling unchanged
* *AND* neither `emit_record_batch_ipc` nor `EmitBatch` MUST be present when the crate is built without the `emit-arrow` feature, and the trait MUST compile in that configuration with no reference to any `arrow` type
* *AND* the row-based `emit(&mut self, values: &[Value])` method MUST remain a required trait method, unchanged, so a UDF MAY freely mix `emit` and `emit_batch` within one `run`
<!-- /DELTA:NEW -->
