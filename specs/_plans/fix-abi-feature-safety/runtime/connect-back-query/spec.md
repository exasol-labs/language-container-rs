# Feature: connect-back-query

The host-side connect-back query path: `RuntimeExaConnection` (backed by exarrow-rs) implementing the SDK's `ExaConnection` trait. This delta removes the `query_arrow` implementation (issue #26) and makes `query_for_each` the streaming primitive the host provides.

## Background

`RuntimeExaConnection` previously implemented `query_arrow`, returning `Vec<arrow::record_batch::RecordBatch>` to the UDF — the issue #26 footgun (Arrow `TypeId` is not stable across the `.so` boundary). With `query_arrow` removed from the trait (see `sdk/connect-back`), the host must implement the now-required `query_for_each` directly: it owns the exarrow-rs connection and the `arrow` crate, converts each batch to `Vec<Value>` host-side, and only `Vec<Value>` rows cross to the UDF. This is also where streaming (bounded memory) is enforced.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: RuntimeExaConnection streams query results as Value rows

* *GIVEN* a `RuntimeExaConnection` wrapping a live exarrow-rs session
* *WHEN* a UDF calls `query_for_each(sql, callback)`
* *THEN* the host MUST execute the query on the dedicated connect-back tokio runtime, obtain the streaming result, and drive it one `RecordBatch` at a time, converting each batch to rows with the single-batch `record_batch_to_rows` helper, invoking the caller's callback once per row with an owned `Vec<Value>`, then dropping the batch and its rows before fetching the next, so peak memory is bounded by one batch
* *AND* the host MUST NOT implement or expose `query_arrow`; `RecordBatch` MUST NOT cross the `.so` boundary — Arrow is confined to the host, and only `Vec<Value>` rows are handed to the UDF (issue #26)
* *AND* `query` MUST be served by the trait default (collecting `query_for_each` into `Vec<Vec<Value>>`), so the materialising and streaming paths share one conversion implementation and cannot diverge
* *AND* the host MUST catch any panic from the async fetch or the conversion and return `UdfError` rather than unwinding across the FFI boundary; if the caller's callback returns an error, `query_for_each` MUST stop fetching further batches and return that error
<!-- /DELTA:CHANGED -->
