# Feature: connect-back

Defines the connect-back surface of the author-facing SDK — the `ConnectionObject` credential struct, the `ExaConnection` trait, and the three `UdfContext` methods — that UDF crates depend on without linking the host runtime or exarrow-rs. The entire surface is feature-gated behind `connect-back`.

## Background

The connect-back surface exposes a public `ConnectionObject` credential struct, an `ExaConnection` trait (referencing no exarrow-rs type), and three `UdfContext` methods: `cluster_ip()` returns the originating cluster node IP, `connection(name)` returns the raw credentials of a named database `CONNECTION` object as a `ConnectionObject`, and `connect_back(&ConnectionObject)` opens a live external-client session. A `ConnectionObject` may also describe a foreign (non-Exasol) system the author drives with another driver. Every session returned by `connect_back` is a new external client session and a new transaction, independent of the invoking query. The `ExaConnection` trait now also exposes transaction control methods (`begin`, `commit`, `rollback`) with default implementations that return `UdfError::Unimplemented` so existing mock implementations continue to compile unchanged.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: ExaConnection trait is defined behind the connect-back feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* the `connect_back` module is referenced
* *THEN* it MUST expose an `ExaConnection` trait with `query_arrow`, `execute`, `begin`, `commit`, and `rollback` methods returning `Result<_, UdfError>`
* *AND* `begin`, `commit`, and `rollback` MUST each have a default implementation that returns `UdfError::Unimplemented`, so connections that do not manage transactions (e.g. test mocks) continue to compile without implementing those methods
* *AND* the `ExaConnection` trait MUST NOT reference any `exarrow-rs` type in its public signature
* *AND* the module MUST NOT expose a `ConnectBackOptions` type, because connection selection is expressed through `ConnectionObject` rather than an options enum
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: ExaConnection transaction defaults return Unimplemented on a mock

* *GIVEN* a type that implements `ExaConnection` without providing `begin`, `commit`, or `rollback`
* *WHEN* any of those three methods is called on the type as `Box<dyn ExaConnection>`
* *THEN* each call MUST return `Err(UdfError::Unimplemented(_))` from the trait default
* *AND* the crate MUST compile with zero errors, confirming the defaults do not require the implementor to supply those methods
<!-- /DELTA:NEW -->
