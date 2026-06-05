# Feature: host-dispatch

Orchestrates loading a UDF `.so`, building the host-side `UdfContext` bridge, and dispatching the database execution model — scalar/set run loops, single-call functions, and connect-back — over the wire protocol.

## Background

The runtime loads a precompiled `.so` (Option A), gating on ABI version and SDK fingerprint, then drives dispatch via the pure protocol state machine. JIT compilation remains unsupported in v2 (`compiler.rs` returns `UnsupportedFeature`). v2 adds single-call dispatch routing `SC_FN_*` to vtable hooks, connect-back via a host implementation of the SDK `ExaConnection` trait over a dedicated `CONNECT_BACK_RT` tokio runtime, and load-time validation of typed `#[exasol_udf(input(...), emits(...))]` schemas against the database metadata.

## Scenarios

<!-- NEW -->
### Scenario: Single-call mode routes to the single-call dispatcher

* *GIVEN* an `MT_META` whose `single_call_mode` is true and whose `single_call_function_id` is `SC_FN_DEFAULT_OUTPUT_COLUMNS`
* *WHEN* the runtime begins dispatch after loading the `.so`
* *THEN* the runtime MUST route to the single-call dispatcher rather than the scalar/set run loop
* *AND* it MUST NOT send `MT_RUN` for that session
<!-- /NEW -->

<!-- NEW -->
### Scenario: Single-call dispatch invokes the matching vtable hook and returns

* *GIVEN* a loaded UDF whose vtable implements `default_output_columns`
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Default_Output_Columns`
* *THEN* it MUST invoke the `default_output_columns` vtable hook with the call payload
* *AND* it MUST reply with `HostAction::SingleCallReturn` carrying the hook result
<!-- /NEW -->

<!-- NEW -->
### Scenario: Unimplemented single-call hook replies MT_UNDEFINED_CALL

* *GIVEN* a loaded UDF whose vtable leaves `generate_sql_for_export_spec` unimplemented
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Generate_Sql_For_Export_Spec`
* *THEN* the hook MUST return `UdfError::Unimplemented`
* *AND* the dispatcher MUST reply with `HostAction::UndefinedCall`
<!-- /NEW -->

<!-- NEW -->
### Scenario: Virtual-schema adapter call is dispatched to the adapter hook

* *GIVEN* a loaded UDF whose vtable implements `virtual_schema_adapter_call`
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Virtual_Schema_Adapter_Call` carrying a request string
* *THEN* it MUST invoke the `virtual_schema_adapter_call` hook with the request payload
* *AND* it MUST reply with `HostAction::SingleCallReturn` carrying the adapter response string
<!-- /NEW -->

<!-- NEW -->
### Scenario: Connect-back opens a connection from the handshake credentials

* *GIVEN* a runtime built with the `connect-back` feature where `UdfMeta` carries connection information from `MT_INFO`
* *WHEN* a UDF first calls `ctx.exa()`
* *THEN* the `HostContextBridge` MUST open an `exarrow-rs` connection on the dedicated `CONNECT_BACK_RT` runtime using the handshake credentials
* *AND* it MUST cache the connection for the remainder of the session
* *AND* a subsequent `ctx.exa()` MUST return the same cached connection without reopening
<!-- /NEW -->

<!-- NEW -->
### Scenario: Connect-back query returns Arrow batches to the UDF

* *GIVEN* a `HostContextBridge` holding an open connect-back connection
* *WHEN* the UDF calls `query_arrow` with a SELECT statement
* *THEN* the host MUST execute the query on the `CONNECT_BACK_RT` runtime and return the result as `Vec<RecordBatch>`
* *AND* a query failure MUST be returned as `UdfError::ConnectBack` rather than panicking
<!-- /NEW -->

<!-- NEW -->
### Scenario: Connect-back retrieves credentials on demand when the handshake carries none

* *GIVEN* a runtime built with the `connect-back` feature where `UdfMeta.conn_info` is `None` because `MT_INFO` carried no connection information
* *AND* the UDF was registered with a `%connection <name>` directive naming a database `CONNECTION` object
* *WHEN* the UDF first calls `ctx.exa()` during `run_batch`
* *THEN* the host MUST send an `MT_IMPORT` request with `kind = PB_IMPORT_CONNECTION_INFORMATION` naming the connection, while the outer dispatch loop is blocked awaiting the function return
* *AND* it MUST build the connect-back connection from the `address`, `user`, and `password` in the `connection_information_rep` response
* *AND* if proactive handshake credentials are present they MUST take priority over the on-demand path
<!-- /NEW -->

<!-- NEW -->
### Scenario: Connect-back connects to the named connection address like an external client

* *GIVEN* a `connection_information_rep` whose `address` is a routable Exasol endpoint and whose `kind` is `password`
* *WHEN* the host opens the connect-back connection
* *THEN* it MUST connect to the `address` exactly as an ordinary external client would, with no assumption of a dedicated internal connect-back proxy
* *AND* it MUST authenticate with the `user` and `password` from the response, not a session token
* *AND* it MUST disable server-certificate validation to match the project transport rule
* *AND* it MUST use the exarrow-rs native binary protocol as the connect-back transport by relying on the exarrow-rs default `native` feature, and MUST NOT set a `transport=websocket` override in the DSN
<!-- /NEW -->

<!-- NEW -->
### Scenario: Annotated schema is validated against the database metadata at load

* *GIVEN* a UDF annotated `#[exasol_udf(input(x: i64), emits(result: i64))]`
* *WHEN* the runtime loads the UDF and compares the annotated schema to the `exascript_metadata` column definitions
* *THEN* a mismatch in column count or `ExaType` MUST close the session with a prefixed `F-UDF-CL-RUST-####` error describing the expected and actual schema
* *AND* a matching schema MUST allow dispatch to proceed
<!-- /NEW -->
