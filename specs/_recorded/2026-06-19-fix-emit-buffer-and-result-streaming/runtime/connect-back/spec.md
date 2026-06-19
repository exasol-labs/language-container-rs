# Feature: connect-back

Implements the host side of the connect-back surface inside the runtime. This change adds a streaming read path: `RuntimeExaConnection::query_for_each` reads the result set one Arrow batch at a time and converts each to rows, instead of materializing the whole result via `Connection::query` / `fetch_all`.

## Background

Connect-back opens a connection from inside the UDF sandbox back to Exasol as an ordinary external client over a dedicated `CONNECT_BACK_RT` tokio runtime. The streaming path uses exarrow-rs `Connection::execute` to obtain a `ResultSet`, turns it into a `ResultSetIterator`, and fetches each `RecordBatch` on the connect-back runtime so the iterator's `Handle::try_current` requirement is met; each batch is converted and dropped before the next fetch.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: query_for_each streams the result set one batch at a time

* *GIVEN* a `RuntimeExaConnection` returned by `connect_back`, wrapping an exarrow-rs `Connection`
* *WHEN* the UDF calls `query_for_each(sql, f)` with a SELECT statement that returns more rows than fit in one exarrow-rs fetch batch
* *THEN* the host MUST execute the query on the dedicated connect-back tokio runtime via `Connection::execute`, obtain the streaming `ResultSet`, and drive it one `RecordBatch` at a time through the result-set iterator rather than calling `fetch_all` / `Connection::query`, which would materialize the entire result set into memory
* *AND* for each fetched batch the host MUST convert it to rows with the single-batch `record_batch_to_rows` helper, invoke the caller's `f` once per row passing an owned `Vec<Value>`, then drop the batch and its rows before fetching the next, so peak memory is bounded by one batch; each per-batch fetch MUST run on the connect-back runtime so the iterator's `Handle::try_current` requirement is satisfied
* *AND* the host MUST catch any panic from the async fetch or the conversion via `catch_unwind` and MUST return `UdfError::ConnectBack` for a panic or a query failure on any batch rather than unwinding across the FFI boundary
* *AND* if the caller's `f` returns an error, `query_for_each` MUST stop fetching further batches and return that error, leaving no further rows processed
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back query returns Arrow batches to the UDF

* *GIVEN* a `RuntimeExaConnection` returned by `connect_back`
* *WHEN* the UDF calls `query_arrow` with a SELECT statement
* *THEN* the host MUST execute the query on the connect-back runtime and return the result as `Vec<RecordBatch>`
* *AND* the host's `query` override MUST be expressed in terms of `query_for_each`, collecting the streamed rows into a `Vec<Vec<Value>>`, so the materializing and streaming paths share one conversion implementation and cannot diverge
* *AND* a query failure MUST be returned as `UdfError::ConnectBack` rather than panicking
<!-- /DELTA:CHANGED -->
