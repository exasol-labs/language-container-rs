# Feature: connect-back

The connect-back surface exposes the public `ConnectionObject` credential struct and the `ExaConnection` trait. This change adds a streaming row API to the trait: `query_for_each` takes a per-row callback and has a default implementation over `query_arrow`, and the row-collecting `query` is re-expressed to delegate to it so the two cannot diverge.

## Background

`ExaConnection` references no exarrow-rs type in its public signature. `query_for_each(sql, f)` invokes `f` once per row across all batches; its default converts batches with `record_batch_to_rows` (the single-batch sibling of `record_batches_to_rows`). Existing impls that provide only `query_arrow` keep working, and `begin`/`commit`/`rollback` keep their `Unimplemented` defaults so mocks compile unchanged.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: ExaConnection trait is defined behind the connect-back feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* the `connect_back` module is referenced
* *THEN* it MUST expose an `ExaConnection` trait with `query_arrow`, `query`, `query_for_each`, `execute`, `begin`, `commit`, and `rollback` methods returning `Result<_, UdfError>`, none of which reference any `exarrow-rs` type in their public signature
* *AND* `query_for_each` MUST take the SQL plus a row callback `F: FnMut(Vec<Value>) -> Result<(), UdfError>` and MUST have a default implementation that delegates to `query_arrow`, converting each batch's rows and invoking the callback, so a connection implementing only `query_arrow` streams without extra code; and `query` MUST have a default that calls `query_for_each` and collects into `Vec<Vec<Value>>`, sharing one code path
* *AND* `begin`, `commit`, and `rollback` MUST each have a default implementation returning `UdfError::Unimplemented`, so connections that do not manage transactions (e.g. test mocks) continue to compile, and the module MUST NOT expose a `ConnectionKind` type because connection selection is expressed through `ConnectionObject`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: query_for_each default streams rows to the callback on a mock connection

* *GIVEN* a type that implements `ExaConnection` by providing only `query_arrow` and `execute`, where `query_arrow` returns two record batches
* *WHEN* `query_for_each` is called on it through the trait default with a callback that pushes each row into a collector
* *THEN* the default MUST invoke the callback once for every row across all batches, in batch-then-row order, passing an owned `Vec<Value>` each time
* *AND* the collected rows MUST equal what `query` returns for the same mock, confirming the two APIs are consistent
* *AND* if the callback returns an error on a given row, `query_for_each` MUST return that error and MUST NOT invoke the callback for any later row
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: record_batch_to_rows converts a single batch without collecting the whole result

* *GIVEN* the `exasol-udf-sdk` connect-back module
* *WHEN* a single `RecordBatch` is passed to the `record_batch_to_rows` helper
* *THEN* it MUST return the rows of exactly that one batch as `Vec<Vec<Value>>`, using the same per-cell type mapping as `record_batches_to_rows`
* *AND* `record_batches_to_rows` MUST be expressed in terms of `record_batch_to_rows` applied per batch, so the multi-batch and single-batch converters cannot diverge in their type handling
<!-- /DELTA:NEW -->
