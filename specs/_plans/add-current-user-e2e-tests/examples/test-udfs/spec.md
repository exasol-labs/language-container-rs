# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

<!-- DELTA:CHANGED -->
Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` (plus `arrow` where needed) and builds as a glibc-dynamic cdylib via a plain host `cargo build --release`. Examples cover core UDF patterns (scalar, set, JSON, typed schema annotation), the iteration-shape contracts (per-row scalar, per-group set, RETURNS via the value-return channel, EMITS via `ctx.emit`, and the negative fixtures that prove runtime gating), multi-entry-point crates, Arrow batch emit, and the handshake identity fields. RETURNS UDFs produce output by returning `Result<Option<T>, UdfError>`; EMITS UDFs produce output via `ctx.emit`. Timestamp fixtures are in `examples/test-udfs-timestamps`; connect-back fixtures are in `examples/test-udfs-connect-back`. The `emit-arrow-batch` fixture crate exercises the `emit-arrow` feature of `exasol-udf-sdk` in isolation (without `connect-back`), serving as the fixture for the live-DB integration suite's Arrow batch-emit path. The `current-user-meta` fixture crate serves the live-DB identity-metadata scenarios in `protocol/handshake`. Every fixture that an integration scenario `dlopen`s MUST be wired into the CI "Build UDF .so artifacts (release)" `-p` allowlist.

A fixture's unit tests build their `UdfContext` double from the `exasol-udf-sdk` `test-support` feature rather than from a hand-written `impl UdfContext`, so a change to the trait's required method set costs one edit in the SDK instead of one edit per fixture.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: current-user-meta reports the session identity fields as one string

* *GIVEN* the `current-user-meta` crate with a `#[exasol_udf]` function whose return type is `Result<Option<String>, UdfError>`
* *WHEN* its `run` reads `ctx.current_user()`, `ctx.scope_user()`, `ctx.current_schema()`, `ctx.script_schema()`, and `ctx.script_name()`
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry_CURRENT_USER_META` with the RETURNS output-shape marker
* *AND* the returned string MUST join the five fields in that order, separated by `|`, with no surrounding whitespace
* *AND* an `Option` accessor that returns `None` MUST render as the literal `<none>`, so an absent database field stays distinguishable from an empty one
* *AND* the crate MUST appear in the workspace `members` and `default-members` lists and in the CI "Build UDF .so artifacts (release)" `-p` allowlist, because an integration scenario `dlopen`s it
<!-- /DELTA:NEW -->
