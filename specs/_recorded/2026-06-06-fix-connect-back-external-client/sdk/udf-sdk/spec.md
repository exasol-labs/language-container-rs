# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, the `#[repr(C)]` ABI vtable, and the connect-back surface — that UDF crates depend on without linking the host runtime or exarrow-rs.

## Background

The connect-back surface is feature-gated. The `ExaConnection` trait exposes `query_arrow` and `execute` and references no exarrow-rs type. Every connection a UDF obtains through `exa`, `exa_named`, or `exa_connect` is a new external client session and a new transaction, independent of the invoking query.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: UdfContext exposes connect-back methods with the feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* a UDF references the `UdfContext` trait
* *THEN* the trait MUST expose `exa(&self) -> Result<&dyn ExaConnection, UdfError>` for the lazy default connection
* *AND* it MUST expose `exa_named(&self, name: &str)` and `exa_connect(&self, opts: ConnectBackOptions)` each returning `Result<Box<dyn ExaConnection>, UdfError>`
* *AND* every connection returned by these methods MUST be a new external client session and a new transaction, independent of the invoking query's session and transaction
* *AND* the `ExaConnection` query methods (`query_arrow`, `execute`) MUST retain their existing signatures, so enabling new-session semantics requires no change to the author-facing API
<!-- /DELTA:CHANGED -->
