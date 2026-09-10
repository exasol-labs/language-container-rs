# Feature: connect-back-query

Implements the host side of the connect-back SELECT/streaming surface inside the runtime: `cluster_ip` parses the originating node IP from the ZMQ endpoint without a network call; `connection` retrieves named-connection credentials via an on-demand `MT_IMPORT` exchange; `connect_back` opens a live `exarrow-rs` session over a dedicated `CONNECT_BACK_RT` tokio runtime. `query_for_each` streams the result set one Arrow batch at a time so peak memory is bounded by one batch; `query` collects via the same path for small, bounded results. `SingleCallContext` exposes the same connect-back methods for VS adapter calls.

## Background

Connect-back opens a connection from inside the UDF sandbox back to Exasol (or any other target) as an ordinary external client. The connect-back surface is three composable `UdfContext` methods: `cluster_ip()` parses the originating node IP from the ZMQ endpoint with no network call; `connection(name)` retrieves the raw credentials of a named database `CONNECTION` object via an on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) exchange and returns a `ConnectionObject`; `connect_back(&ConnectionObject)` opens a live `exarrow-rs` session to the target as an ordinary external client over the native binary protocol with server-certificate validation disabled. The MT_IMPORT exchange is safe during the run phase because the outer dispatch loop is blocked awaiting the UDF function return, so the ZMQ socket is idle.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: query_for_each streams the result set one batch at a time

* *GIVEN* a `RuntimeExaConnection` returned by `connect_back`, wrapping an exarrow-rs `Connection`
* *WHEN* the UDF calls `query_for_each(sql, f)` with a SELECT statement that returns more rows than fit in one exarrow-rs fetch batch
* *THEN* the host MUST drive the whole fetch inside one `block_on` of the dedicated connect-back tokio runtime, awaiting each `RecordBatch` in turn, and MUST NOT call `fetch_all` or `Connection::query`, both of which materialise the entire result set in memory before any row reaches the caller
* *AND* it MUST convert each awaited batch with `record_batch_to_rows`, invoke `f` once per row from inside that async block, then drop the batch before awaiting the next
* *AND* `RecordBatch` MUST NOT cross the `.so` boundary; conversion MUST run in the runtime crate
* *AND* `query` MUST collect the rows this method yields rather than carrying a second conversion implementation
* *AND* if `f` returns an error, `query_for_each` MUST stop awaiting further batches and return that error
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: RuntimeExaConnection streams query results as Value rows

* *GIVEN* a `RuntimeExaConnection` wrapping a live exarrow-rs session
* *WHEN* a UDF calls `query_for_each(sql, callback)`
* *THEN* this scenario MUST be removed as a duplicate: it restates the streaming contract of "query_for_each streams the result set one batch at a time" in different mechanism words, and its unique clauses (no `query_arrow`, no `RecordBatch` across the `.so` boundary, `query` collecting the streaming path) are folded into that scenario
<!-- /DELTA:REMOVED -->
</content>
