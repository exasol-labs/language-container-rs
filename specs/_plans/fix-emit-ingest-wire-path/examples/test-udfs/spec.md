# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` (plus `arrow` where needed) and builds as a glibc-dynamic cdylib via a plain host `cargo build --release`. Examples cover core UDF patterns (scalar, set, JSON, typed schema annotation), the iteration-shape contracts (per-row scalar, per-group set, RETURNS via the value-return channel, EMITS via `ctx.emit`, and the negative fixtures that prove runtime gating), multi-entry-point crates, Arrow batch emit, and the handshake identity fields. RETURNS UDFs produce output by returning `Result<Option<T>, UdfError>`; EMITS UDFs produce output via `ctx.emit`. Timestamp fixtures are in `examples/test-udfs-timestamps`; connect-back fixtures are in `examples/test-udfs-connect-back`. The `emit-arrow-batch` fixture crate exercises the `emit-arrow` feature of `exasol-udf-sdk` in isolation (without `connect-back`), serving as the fixture for the live-DB integration suite's Arrow batch-emit path. The `current-user-meta` fixture crate serves the live-DB identity-metadata scenarios in `protocol/handshake`. Every fixture that an integration scenario `dlopen`s MUST be wired into the CI "Build UDF .so artifacts (release)" `-p` allowlist.

A fixture's unit tests build their `UdfContext` double from the `exasol-udf-sdk` `test-support` feature rather than from a hand-written `impl UdfContext`, so a change to the trait's required method set costs one edit in the SDK instead of one edit per fixture.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: emit-k emits a variable number of rows per input row

* *GIVEN* the `emit-k` crate with a `#[exasol_udf]` function whose return type is `Result<(), UdfError>`, whose `run` reads the first `i64` column `k` of the current row and calls `ctx.emit_owned()` `k` times
* *WHEN* the crate is registered as `RUST SCALAR SCRIPT ... EMITS (v BIGINT)` and invoked over rows with `k = 0`, `k = 1`, and `k = N > 1`
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry_EMIT_K` with the EMITS output-shape marker
* *AND* each input row MUST produce exactly `k` output rows, proving a SCALAR EMITS UDF supports zero, one, and many emits per input row
* *AND* the fixture MUST call the owned-row entry point `emit_owned`, so the integration suite dispatches that vtable slot across a real `dlopen` boundary and a slot-order defect in the widened `UdfContext` vtable cannot hide
<!-- /DELTA:CHANGED -->
</content>
