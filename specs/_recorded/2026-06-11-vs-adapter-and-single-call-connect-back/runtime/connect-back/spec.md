# Feature: connect-back

Implements the host side of the connect-back surface inside the runtime: `cluster_ip` parses the originating node IP from the ZMQ endpoint without a network call; `connection` retrieves named-connection credentials via an on-demand `MT_IMPORT` exchange; `connect_back` opens a live `exarrow-rs` session over a dedicated `CONNECT_BACK_RT` tokio runtime. The runtime also implements `begin`, `commit`, and `rollback` on `RuntimeExaConnection` and provides `SingleCallContext` so VS adapter calls can resolve credentials and open connect-back sessions mid single-call.

## Background

Connect-back opens a connection from inside the UDF sandbox back to Exasol (or any other target) as an ordinary external client. The connect-back surface is three composable `UdfContext` methods: `cluster_ip()` parses the originating node IP from the ZMQ endpoint with no network call; `connection(name)` retrieves the raw credentials of a named database `CONNECTION` object via an on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) exchange and returns a `ConnectionObject`; `connect_back(&ConnectionObject)` opens a live `exarrow-rs` session to the target as an ordinary external client over the native binary protocol with server-certificate validation disabled. The MT_IMPORT exchange is safe during the run phase because the outer dispatch loop is blocked awaiting the UDF function return, so the ZMQ socket is idle. Transaction control (`begin`/`commit`/`rollback`) is driven on the same `CONNECT_BACK_RT` tokio runtime via `block_on`, with `catch_unwind` to prevent panics from crossing the FFI boundary.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: RuntimeExaConnection implements begin, commit, and rollback

* *GIVEN* a `Box<dyn ExaConnection>` returned by `ctx.connect_back` (a `RuntimeExaConnection` under the hood)
* *WHEN* the UDF calls `begin()`, `commit()`, or `rollback()` on the connection
* *THEN* each call MUST drive the corresponding `exarrow_rs::Connection` operation on the dedicated `CONNECT_BACK_RT` tokio runtime via `block_on`
* *AND* an `exarrow_rs::QueryError` from the operation MUST be mapped to `UdfError::ConnectBack(e.to_string())`
* *AND* a panic inside `block_on` MUST be caught by `catch_unwind` and returned as `UdfError::ConnectBack("panic in <op>: <payload>")` rather than unwinding across the FFI boundary

### Scenario: SingleCallContext exposes connect-back methods for VS adapter calls

* *GIVEN* a runtime dispatching `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL` with the `connect-back` feature enabled
* *WHEN* the VS adapter function calls `ctx.cluster_ip()`, `ctx.connection(name)`, or `ctx.connect_back(conn)` on the `&mut dyn UdfContext` it receives
* *THEN* `cluster_ip()` MUST return the first non-loopback IPv4 address (same algorithm as `HostContextBridge`)
* *AND* `connection(name)` MUST perform an on-demand `MT_IMPORT` exchange on the ZMQ socket and return a `ConnectionObject`
* *AND* `connect_back(conn)` MUST open a new `exarrow-rs` session and return a `Box<dyn ExaConnection>` owned by the UDF
* *AND* the data methods `get`, `emit`, and `next` MUST return `UdfError::Unimplemented` because single-call mode exchanges one JSON string, not row batches

### Scenario: SingleCallContext connection method performs MT_IMPORT while socket is idle

* *GIVEN* a `SingleCallContext` built with a `ConnRequester` closure that drives the ZMQ socket
* *WHEN* the VS adapter calls `ctx.connection("MY_CONN")` during the `virtual_schema_adapter_call` hook
* *THEN* the host MUST send `MT_IMPORT` with `kind = PB_IMPORT_CONNECTION_INFORMATION` and `script_name = "MY_CONN"` over the idle socket
* *AND* the returned `ConnInfo` MUST be mapped to a `ConnectionObject` and returned to the adapter
* *AND* the dispatch loop MUST NOT observe this exchange because it is blocked awaiting the hook's return value — the socket is not concurrently accessed
<!-- /DELTA:NEW -->
