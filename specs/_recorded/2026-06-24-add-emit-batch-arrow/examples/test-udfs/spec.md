# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests. This delta adds an Arrow batch-emit fixture.

## Background

This delta adds the `emit-arrow-batch` fixture crate, a standalone cdylib depending on `exasol-udf-sdk` with the `emit-arrow` feature (not `connect-back`). It builds a `RecordBatch` in `run` and emits it via `ctx.emit_batch`, proving the Arrow batch-emit path end-to-end for the `integration/db-roundtrip` suite.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: emit-arrow-batch emits a manually built Arrow RecordBatch

* *GIVEN* the `emit-arrow-batch` crate built against `exasol-udf-sdk` with the `emit-arrow` feature, with a `#[exasol_udf]` SET entry point implementing `UdfRun`
* *WHEN* its `run` drains the input rows, builds an `arrow` `RecordBatch` whose columns match the EMITS output schema (e.g. an `Int64` column and a `Utf8` column), and calls `ctx.emit_batch(&batch)` once
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting the named entry point derived from the function identifier
* *AND* the crate MUST enable only the `emit-arrow` feature on `exasol-udf-sdk` (NOT `connect-back`), proving Arrow batch emit works as a standalone capability
* *AND* every row of the emitted `RecordBatch` MUST reach the output unchanged, in batch order, so the batch-emit path is observably equivalent to emitting the same rows via row-based `emit`
<!-- /DELTA:NEW -->
