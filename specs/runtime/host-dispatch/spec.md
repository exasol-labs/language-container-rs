# Feature: host-dispatch

Orchestrates loading a UDF `.so`, building the host-side `UdfContext` bridge, and dispatching the database execution model — scalar/set run loops, single-call functions, and connect-back — over the wire protocol.

## Background

Connect-back opens a connection from inside the UDF sandbox back to Exasol. The host fetches credentials via `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) — a pure metadata retrieval equivalent to PyExasol's `exa.get_connection(NAME)` — then connects to the returned `address` as an ordinary external client over the exarrow-rs native binary protocol with server-certificate validation disabled.

The runtime loads a precompiled `.so` (Option A), gating on ABI version and SDK fingerprint, then drives dispatch via the pure protocol state machine. JIT compilation remains unsupported in v2 (`compiler.rs` returns `UnsupportedFeature`). v2 adds single-call dispatch routing `SC_FN_*` to vtable hooks, connect-back via a host implementation of the SDK `ExaConnection` trait over a dedicated `CONNECT_BACK_RT` tokio runtime, and load-time validation of typed `#[exasol_udf(input(...), emits(...))]` schemas against the database metadata.

## Scenarios

### Scenario: Loader accepts a matching .so and calls create

* *GIVEN* a UDF `.so` built against the host's SDK fingerprint and `abi_version = 1`
* *WHEN* the loader opens it and resolves `__exa_udf_entry`
* *THEN* it MUST verify `abi_version` equals `EXA_UDF_ABI_VERSION`
* *AND* it MUST verify the vtable `sdk_fingerprint` matches the host fingerprint
* *AND* it MUST call `create` and return a handle holding the `Library` alive

### Scenario: Loader rejects an ABI version mismatch

* *GIVEN* a UDF `.so` whose vtable reports an `abi_version` other than `1`
* *WHEN* the loader validates the vtable
* *THEN* it MUST return a clear error identifying the version mismatch
* *AND* it MUST NOT call `create` or dereference any function pointers

### Scenario: Loader rejects a fingerprint mismatch

* *GIVEN* a UDF `.so` whose vtable `sdk_fingerprint` does not match the host fingerprint
* *WHEN* the loader validates the vtable
* *THEN* it MUST return a clear error identifying the fingerprint mismatch rather than producing undefined behavior
* *AND* it MUST NOT call `create`

### Scenario: Artifact path is parsed from the udf_object option

* *GIVEN* a script source containing `%udf_object /buckets/bfsdefault/default/udfs/libudf.so`
* *WHEN* the runtime resolves the artifact
* *THEN* it MUST extract the `.so` path from the `%udf_object` option
* *AND* it MUST load that path via the loader without invoking the JIT compiler

### Scenario: JIT compilation is unsupported in v1

* *GIVEN* a script source with no `%udf_object` option (JIT path)
* *WHEN* the runtime attempts to resolve the artifact
* *THEN* the compiler entry point MUST return an unsupported-feature error
* *AND* the error MUST be surfaced through the protocol close path with the `F-UDF-CL-RUST-` prefix

### Scenario: Bridge materializes input rows into typed accessors

* *GIVEN* a `HostContextBridge` over a fake transport delivering one input batch of mixed column types
* *WHEN* a UDF calls `next` then the typed accessors
* *THEN* `next` MUST return `true` while rows remain and `false` when input is exhausted
* *AND* each typed accessor MUST return the correct value for the current row, mapping the eight PB column types to their Arrow arrays
* *AND* a NULL cell MUST be returned as `None`

### Scenario: Scalar dispatch runs the UDF and emits one batch

* *GIVEN* a loaded scalar UDF and a `HostContextBridge` with `iter_type = ExactlyOnce`
* *WHEN* the runtime sends `MT_RUN` and calls the vtable `run`
* *THEN* the bridge `next`/`emit` calls MUST drive `MT_NEXT`/`MT_EMIT` exchanges against the transport
* *AND* on `run` return the runtime MUST send `MT_DONE`

### Scenario: Set/EMITS dispatch emits multiple rows across batches

* *GIVEN* a loaded set UDF and a `HostContextBridge` with `iter_type = Multiple`
* *WHEN* the UDF iterates all input rows and emits a filtered subset
* *THEN* `emit` MUST accumulate output and flush `MT_EMIT` in batches rather than one frame per row
* *AND* the total emitted row count MUST equal the number of `emit` calls the UDF made

### Scenario: UDF error closes the session with a prefixed message

* *GIVEN* a loaded UDF whose `run` returns a non-zero error code
* *WHEN* the runtime observes the failure
* *THEN* it MUST serialize the error message into the protocol close path with the `F-UDF-CL-RUST-` prefix
* *AND* it MUST call the vtable `destroy` and drop the `Library` before returning failure

### Scenario: Single-call mode routes to the single-call dispatcher

* *GIVEN* an `MT_META` whose `single_call_mode` is true and whose `single_call_function_id` is `SC_FN_DEFAULT_OUTPUT_COLUMNS`
* *WHEN* the runtime begins dispatch after loading the `.so`
* *THEN* the runtime MUST route to the single-call dispatcher rather than the scalar/set run loop
* *AND* it MUST NOT send `MT_RUN` for that session

### Scenario: Single-call dispatch invokes the matching vtable hook and returns

* *GIVEN* a loaded UDF whose vtable implements `default_output_columns`
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Default_Output_Columns`
* *THEN* it MUST invoke the `default_output_columns` vtable hook with the call payload
* *AND* it MUST reply with `HostAction::SingleCallReturn` carrying the hook result

### Scenario: Unimplemented single-call hook replies MT_UNDEFINED_CALL

* *GIVEN* a loaded UDF whose vtable leaves `generate_sql_for_export_spec` unimplemented
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Generate_Sql_For_Export_Spec`
* *THEN* the hook MUST return `UdfError::Unimplemented`
* *AND* the dispatcher MUST reply with `HostAction::UndefinedCall`

### Scenario: Virtual-schema adapter call is dispatched to the adapter hook

* *GIVEN* a loaded UDF whose vtable implements `virtual_schema_adapter_call`
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Virtual_Schema_Adapter_Call` carrying a request string
* *THEN* it MUST invoke the `virtual_schema_adapter_call` hook with the request payload
* *AND* it MUST reply with `HostAction::SingleCallReturn` carrying the adapter response string

### Scenario: Connect-back opens a connection from the handshake credentials

* *GIVEN* a runtime built with the `connect-back` feature where `UdfMeta` carries connection information from `MT_INFO`
* *WHEN* a UDF first calls `ctx.exa()`
* *THEN* the `HostContextBridge` MUST open an `exarrow-rs` connection on the dedicated `CONNECT_BACK_RT` runtime using the handshake credentials
* *AND* it MUST cache the connection for the remainder of the session
* *AND* a subsequent `ctx.exa()` MUST return the same cached connection without reopening

### Scenario: Connect-back query returns Arrow batches to the UDF

* *GIVEN* a `HostContextBridge` holding an open connect-back connection
* *WHEN* the UDF calls `query_arrow` with a SELECT statement
* *THEN* the host MUST execute the query on the `CONNECT_BACK_RT` runtime and return the result as `Vec<RecordBatch>`
* *AND* a query failure MUST be returned as `UdfError::ConnectBack` rather than panicking

### Scenario: Connect-back retrieves credentials on demand when the handshake carries none

* *GIVEN* a runtime built with the `connect-back` feature where `UdfMeta.conn_info` is `None` because `MT_INFO` carried no connection information
* *AND* the UDF was registered with a `%connection <name>` directive naming a database `CONNECTION` object
* *WHEN* the UDF first calls `ctx.exa()` during `run_batch`
* *THEN* the host MUST send an `MT_IMPORT` request with `kind = PB_IMPORT_CONNECTION_INFORMATION` naming the connection, while the outer dispatch loop is blocked awaiting the function return
* *AND* it MUST build the connect-back connection from the `address`, `user`, and `password` in the `connection_information_rep` response
* *AND* if proactive handshake credentials are present they MUST take priority over the on-demand path

### Scenario: Connect-back connects to the named connection address like an external client

* *GIVEN* a `connection_information_rep` whose `address` is a routable Exasol endpoint and whose `kind` is `password`
* *WHEN* the host opens the connect-back connection
* *THEN* it MUST connect to the `address` exactly as an ordinary external client would, opening a new database session and a new transaction, authenticated with the `user` and `password` from the response and not a session token
* *AND* it MUST NOT attempt to share or join the invoking query's session or transaction, because the Exasol core cannot share a transaction with a language-container UDF
* *AND* it MUST disable server-certificate validation to match the project transport rule
* *AND* it MUST use the exarrow-rs native binary protocol by relying on the default `native` feature, and MUST NOT set a `transport=websocket` override in the DSN

### Scenario: Connect-back named connection makes the UDF portable across clusters

* *GIVEN* a UDF script registered with a generic `%connection <NAME>` directive and no hardcoded cluster address in its source
* *AND* a database `CREATE CONNECTION <NAME> TO '<cluster-address>:8563' USER '...' IDENTIFIED BY '...'` object that the operator populated with the correct address for the target cluster
* *WHEN* the host requests credentials with `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) naming `<NAME>` and receives the `connection_information_rep`
* *THEN* the host MUST build the connect-back DSN solely from the `address`, `user`, and `password` returned for that named connection
* *AND* the host MUST NOT embed or assume any cluster-specific address of its own, so the same UDF artifact remains portable across clusters that differ only in the `CREATE CONNECTION` definition

### Scenario: Annotated schema is validated against the database metadata at load

* *GIVEN* a UDF annotated `#[exasol_udf(input(x: i64), emits(result: i64))]`
* *WHEN* the runtime loads the UDF and compares the annotated schema to the `exascript_metadata` column definitions
* *THEN* a mismatch in column count or `ExaType` MUST close the session with a prefixed `F-UDF-CL-RUST-####` error describing the expected and actual schema
* *AND* a matching schema MUST allow dispatch to proceed
