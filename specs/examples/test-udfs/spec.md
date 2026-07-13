# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` (plus `arrow` where needed) and builds for the `x86_64-unknown-linux-musl` target. Examples cover core UDF patterns (scalar, set, JSON, typed schema annotation), the iteration-shape contracts (per-row scalar, per-group set, RETURNS via the value-return channel, EMITS via `ctx.emit`, and the negative fixtures that prove runtime gating), multi-entry-point crates, and Arrow batch emit. RETURNS UDFs produce output by returning `Result<Option<T>, UdfError>`; EMITS UDFs produce output via `ctx.emit`. Timestamp fixtures are in `examples/test-udfs-timestamps`; connect-back fixtures are in `examples/test-udfs-connect-back`. The `emit-arrow-batch` fixture crate exercises the `emit-arrow` feature of `exasol-udf-sdk` in isolation (without `connect-back`), serving as the fixture for the live-DB integration suite's Arrow batch-emit path. Every fixture that an integration scenario `dlopen`s MUST be wired into the CI "Build UDF .so artifacts (release)" `-p` allowlist.

## Scenarios

### Scenario: scalar-double returns twice its input

* *GIVEN* the `scalar-double` crate with a `#[exasol_udf]` function whose return type is `Result<Option<Value>, UdfError>`
* *WHEN* its `run` reads the first column as `i64` and returns `Ok(Some(Value::Int64(x * 2)))`, or `Ok(None)` for a NULL input
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry_SCALAR_DOUBLE` with the RETURNS output-shape marker
* *AND* for input `21` the returned value MUST be `42`, and a NULL input MUST return SQL NULL

### Scenario: set-filter emits only positive rows

* *GIVEN* the `set-filter` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` loops `ctx.next()` and emits each row whose first `i64` column is greater than zero
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry_SET_FILTER`
* *AND* non-positive rows MUST NOT be emitted

### Scenario: json-parse extracts a field using serde_json

* *GIVEN* the `json-parse` crate depending on `serde_json` with a `#[exasol_udf]` function whose return type is `Result<Option<String>, UdfError>`
* *WHEN* its `run` reads the first column as a string, parses it with `serde_json`, and returns the `name` field
* *THEN* the crate MUST compile to a cdylib for `x86_64-unknown-linux-musl` with `serde_json` statically linked and the RETURNS output-shape marker
* *AND* for input `{"name":"exa"}` the returned value MUST be `exa`, and a NULL input MUST return SQL NULL

### Scenario: annotated-double declares its schema via the typed annotation

* *GIVEN* an example UDF `fn annotated_double` annotated `#[exasol_udf(input(x: Decimal), emits(result: Decimal))]`
* *WHEN* the example is built for the musl target
* *THEN* the generated vtable MUST embed the input column `x: Numeric` and emit column `result: Numeric`
* *AND* the artifact MUST export the named entry point `__exa_udf_entry_ANNOTATED_DOUBLE` derived from the function identifier
* *AND* the example MUST double its input as in `scalar-double`

### Scenario: annotated-fixture exports two named entry points from one .so

* *GIVEN* the `annotated-fixture` crate declaring two annotated functions `fn annotated` and `fn annotated_double` in the same crate, each `#[exasol_udf]`
* *WHEN* the crate is built as a cdylib for the musl target
* *THEN* the single artifact MUST export both `__exa_udf_entry_ANNOTATED` and `__exa_udf_entry_ANNOTATED_DOUBLE`
* *AND* each entry point MUST return its own `*const ExaUdfVTable` with the matching annotated schema
* *AND* the build MUST succeed without a duplicate-symbol link error

### Scenario: emit-arrow-batch emits a manually built Arrow RecordBatch

* *GIVEN* the `emit-arrow-batch` crate built against `exasol-udf-sdk` with the `emit-arrow` feature, with a `#[exasol_udf]` SET entry point implementing `UdfRun`
* *WHEN* its `run` drains the input rows, builds an `arrow` `RecordBatch` whose columns match the EMITS output schema (e.g. an `Int64` column and a `Utf8` column), and calls `ctx.emit_batch(&batch)` once
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting the named entry point derived from the function identifier
* *AND* the crate MUST enable only the `emit-arrow` feature on `exasol-udf-sdk` (NOT `connect-back`), proving Arrow batch emit works as a standalone capability
* *AND* every row of the emitted `RecordBatch` MUST reach the output unchanged, in batch order, so the batch-emit path is observably equivalent to emitting the same rows via row-based `emit`

### Scenario: set-sum aggregates a group and returns one value

* *GIVEN* the `set-sum` crate with a `#[exasol_udf]` function whose return type is `Result<Option<Value>, UdfError>`, registered as `RUST SET SCRIPT ... RETURNS BIGINT`
* *WHEN* its `run` loops `ctx.next()` over every row of the current input group, accumulating the first `i64` column, and returns `Ok(Some(Value::Int64(sum)))`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry_SET_SUM` with the RETURNS output-shape marker
* *AND* for a group whose rows sum to `S` the returned value MUST equal `S`, independent of how many `MT_NEXT` batches the group spans

### Scenario: emit-k emits a variable number of rows per input row

* *GIVEN* the `emit-k` crate with a `#[exasol_udf]` function whose return type is `Result<(), UdfError>`, whose `run` reads the first `i64` column `k` of the current row and calls `ctx.emit()` `k` times
* *WHEN* the crate is registered as `RUST SCALAR SCRIPT ... EMITS (v BIGINT)` and invoked over rows with `k = 0`, `k = 1`, and `k = N > 1`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry_EMIT_K` with the EMITS output-shape marker
* *AND* each input row MUST produce exactly `k` output rows, proving a SCALAR EMITS UDF supports zero, one, and many emits per input row
