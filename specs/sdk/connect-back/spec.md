# Feature: connect-back

Defines the connect-back surface of the author-facing SDK — the `ConnectionObject` credential struct, the `ExaConnection` trait, and the three `UdfContext` methods — that UDF crates depend on without linking the host runtime or exarrow-rs. The entire surface is feature-gated behind `connect-back`.

## Background

The connect-back surface exposes a public `ConnectionObject` credential struct, an `ExaConnection` trait (referencing no exarrow-rs type), and three `UdfContext` methods: `cluster_ip()` returns the originating cluster node IP, `connection(name)` returns the raw credentials of a named database `CONNECTION` object as a `ConnectionObject`, and `connect_back(&ConnectionObject)` opens a live external-client session. A `ConnectionObject` may also describe a foreign (non-Exasol) system the author drives with another driver. Every session returned by `connect_back` is a new external client session and a new transaction, independent of the invoking query. The `ExaConnection` trait now also exposes transaction control methods (`begin`, `commit`, `rollback`) with default implementations that return `UdfError::Unimplemented` so existing mock implementations continue to compile unchanged.

## Scenarios

### Scenario: ConnectionObject is a public connect-back SDK type

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* the `connect_back` module is referenced
* *THEN* it MUST expose a public `ConnectionObject` struct with public `kind`, `address`, `user`, and `password` `String` fields
* *AND* `ConnectionObject` MUST mirror the four fields of a database `CONNECTION` object so a UDF author can read or construct credentials for either an Exasol or a foreign target
* *AND* the `ConnectionObject` type MUST NOT reference any transport-layer type (it MUST NOT re-export or alias `exa-zmq-protocol`'s internal `ConnInfo`)

### Scenario: ExaConnection trait is defined behind the connect-back feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* the `connect_back` module is referenced
* *THEN* it MUST expose an `ExaConnection` trait with `query_arrow`, `query`, `query_for_each`, `execute`, `begin`, `commit`, and `rollback` methods returning `Result<_, UdfError>`, none of which reference any `exarrow-rs` type in their public signature
* *AND* `query_for_each` MUST take the SQL plus a row callback `F: FnMut(Vec<Value>) -> Result<(), UdfError>` and MUST have a default implementation that delegates to `query_arrow`, converting each batch's rows and invoking the callback, so a connection implementing only `query_arrow` streams without extra code; and `query` MUST have a default that calls `query_for_each` and collects into `Vec<Vec<Value>>`, sharing one code path
* *AND* `begin`, `commit`, and `rollback` MUST each have a default implementation returning `UdfError::Unimplemented`, so connections that do not manage transactions (e.g. test mocks) continue to compile, and the module MUST NOT expose a `ConnectionKind` type because connection selection is expressed through `ConnectionObject`

### Scenario: UdfContext connect-back methods are absent without the feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature disabled
* *WHEN* the crate is compiled
* *THEN* the `UdfContext` methods `cluster_ip`, `connection`, and `connect_back` MUST NOT be present
* *AND* the `ConnectionObject` and `ExaConnection` types MUST NOT be present
* *AND* the crate MUST NOT depend on `tokio` or `exarrow-rs`

### Scenario: UdfContext exposes connect-back methods with the feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* a UDF references the `UdfContext` trait
* *THEN* the trait MUST expose `cluster_ip(&self) -> Result<String, UdfError>` returning the IP of the cluster node that started the language container
* *AND* it MUST expose `connection(&self, name: &str) -> Result<ConnectionObject, UdfError>` returning the raw credentials of the named database `CONNECTION` object
* *AND* it MUST expose `connect_back(&mut self, conn: &ConnectionObject) -> Result<Box<dyn ExaConnection>, UdfError>` opening a live `exarrow-rs` session that is a new external client session and a new transaction, independent of the invoking query's session and transaction
* *AND* each of the three methods MUST have a default implementation returning `UdfError::Unimplemented` so a `UdfContext` impl that does not support connect-back still compiles

### Scenario: connect_back accepts a caller-built ConnectionObject for a foreign target

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *AND* a `ConnectionObject` a UDF author constructed directly rather than obtaining it from `connection`
* *WHEN* the UDF calls `connect_back` with that object
* *THEN* the call MUST build the live session solely from the `address`, `user`, and `password` of the passed `ConnectionObject`
* *AND* it MUST NOT require the object to have originated from a database `CONNECTION`, so a UDF MAY pair `cluster_ip` with credentials read from `connection` to target the cluster node explicitly

### Scenario: ExaConnection transaction defaults return Unimplemented on a mock

* *GIVEN* a type that implements `ExaConnection` without providing `begin`, `commit`, or `rollback`
* *WHEN* any of those three methods is called on the type as `Box<dyn ExaConnection>`
* *THEN* each call MUST return `Err(UdfError::Unimplemented(_))` from the trait default
* *AND* the crate MUST compile with zero errors, confirming the defaults do not require the implementor to supply those methods

### Scenario: query_for_each default streams rows to the callback on a mock connection

* *GIVEN* a type that implements `ExaConnection` by providing only `query_arrow` and `execute`, where `query_arrow` returns two record batches
* *WHEN* `query_for_each` is called on it through the trait default with a callback that pushes each row into a collector
* *THEN* the default MUST invoke the callback once for every row across all batches, in batch-then-row order, passing an owned `Vec<Value>` each time
* *AND* the collected rows MUST equal what `query` returns for the same mock, confirming the two APIs are consistent
* *AND* if the callback returns an error on a given row, `query_for_each` MUST return that error and MUST NOT invoke the callback for any later row

### Scenario: record_batch_to_rows converts a single batch without collecting the whole result

* *GIVEN* the `exasol-udf-sdk` connect-back module
* *WHEN* a single `RecordBatch` is passed to the `record_batch_to_rows` helper
* *THEN* it MUST return the rows of exactly that one batch as `Vec<Vec<Value>>`, using the same per-cell type mapping as `record_batches_to_rows`
* *AND* `record_batches_to_rows` MUST be expressed in terms of `record_batch_to_rows` applied per batch, so the multi-batch and single-batch converters cannot diverge in their type handling
