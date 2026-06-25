# Feature: connect-back

Defines the connect-back surface of the author-facing SDK — the `ConnectionObject` credential struct, the `ExaConnection` trait, and the `UdfContext` connect-back methods. This delta removes the unsafe `query_arrow` (issue #26) and makes the connect-back types compile unconditionally so the `UdfContext` vtable can be feature-independent (issue #31).

## Background

The connect-back surface was gated behind a `connect-back` SDK feature, and `ExaConnection` exposed `query_arrow` returning `Vec<arrow::record_batch::RecordBatch>`. That signature is unsafe across the `.so` boundary: a UDF `.so` and the host each link their own static `arrow`, so downcasts on those arrays silently return `None` (mismatched `TypeId`/vtables) — wrong values, no error (issue #26).

Removing `query_arrow` makes `ExaConnection` **arrow-free**, which lets the entire `connect_back` module (and `ConnectionObject`/`ExaConnection`) compile **unconditionally** — no SDK `connect-back` feature. That is the prerequisite for issue #31's fix: with the connect-back types always present, `UdfContext` can declare its `connection`/`connect_back` methods unconditionally, giving a feature-independent trait-object vtable. Arrow remains the host's internal transport (exarrow-rs → batches → `Vec<Value>`); UDFs only ever receive `Vec<Value>`.

## Scenarios

### Scenario: ConnectionObject is a public connect-back SDK type

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* the `connect_back` module is referenced
* *THEN* it MUST expose a public `ConnectionObject` struct with public `kind`, `address`, `user`, and `password` fields, available unconditionally (no feature gate)
* *AND* the `ConnectionObject` type MUST NOT reference any transport-layer type (it MUST NOT re-export or alias exarrow-rs internals)

### Scenario: ExaConnection trait is arrow-free and always compiled

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* the `connect_back` module is referenced
* *THEN* the `connect_back` module and the public `ConnectionObject` and `ExaConnection` items MUST compile **unconditionally** (no `#[cfg(feature = "connect-back")]` gate) and the crate MUST NOT define a `connect-back` cargo feature, because the trait no longer references any `arrow` type and so needs no optional dependency
* *AND* `ExaConnection` MUST expose `query`, `query_for_each`, `execute`, `execute_batch`, `begin`, `commit`, and `rollback`, all returning `Result<_, UdfError>`, none of which reference any `arrow` or `exarrow-rs` type in their public signature
* *AND* the trait MUST NOT declare `query_arrow` (or any method returning `Vec<RecordBatch>` / `Arc<dyn Array>` across the `.so` boundary), because that `repr(Rust)` Arrow type is not ABI-stable across two independently linked static `arrow` copies (issue #26)
* *AND* `query_for_each` MUST be a required method taking the SQL plus a row callback `F: FnMut(Vec<Value>) -> Result<(), UdfError>`, and `query` MUST default to calling `query_for_each` and collecting into `Vec<Vec<Value>>`, so both share one code path and neither depends on a boundary-crossing Arrow type
* *AND* `execute_batch` (accepting `sql: &str`, `rows: &[Vec<Value>]`), `begin`, `commit`, and `rollback` MUST each have a default implementation returning `UdfError::Unimplemented`, so mocks and connections that do not support them continue to compile unchanged

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

### Scenario: ExaConnection execute_batch default returns Unimplemented on a mock

* *GIVEN* a type that implements `ExaConnection` without providing `execute_batch`
* *WHEN* `execute_batch(sql, rows)` is called on the type as `Box<dyn ExaConnection>`
* *THEN* the call MUST return `Err(UdfError::Unimplemented(_))` from the trait default
* *AND* the crate MUST compile with zero errors, confirming the default does not require the implementor to supply `execute_batch`
* *AND* the `execute_batch` signature MUST be `fn execute_batch(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<u64, UdfError>` with no `exarrow-rs` type in the public signature

### Scenario: query_arrow is removed from the cross-boundary trait surface

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* a UDF or a mock references the `ExaConnection` trait
* *THEN* the trait MUST NOT expose `query_arrow` (issue #26 footgun) nor any method handing back a `repr(Rust)` Arrow type across the `.so` boundary
* *AND* tests and mocks that previously implemented `query_arrow` MUST instead implement `query_for_each` (now required) to supply their rows, so the trait stays object-safe and the `Value`-based default (`query`) keeps working
* *AND* the SDK MUST document that connect-back results are delivered only as `Vec<Value>` rows; Arrow is the host's internal transport and never crosses to UDF code
