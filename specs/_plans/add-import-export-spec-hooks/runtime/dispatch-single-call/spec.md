# Feature: dispatch-single-call

Handles the single-call dispatch path — routing `SC_FN_*` function IDs from `MT_META` to the matching vtable hooks, replying with `MT_RETURN` or `MT_UNDEFINED_CALL`, and validating annotated UDF schemas against database metadata at load time. The scalar/set run loop is specified separately in `runtime/dispatch-run-loop`.

<!-- DELTA:CHANGED -->
## Background

The runtime routes `SC_FN_*` to vtable hooks and validates typed `#[exasol_udf(input(...), emits(...))]` schemas against the database metadata at load time.

Every hook that can need host state receives a `SingleCallContext` through the double-indirected `*mut c_void` ABI: `virtual_schema_adapter_call`, `generate_sql_for_import_spec`, and `generate_sql_for_export_spec`. The context carries the `exascript_info` handshake metadata threaded in from `MT_META` and resolves CONNECTION credentials over an on-demand `MT_IMPORT` exchange, so a hook sees the same live identity, origin, and topology values the scalar/set run loop's `HostContextBridge` surfaces. It reports no rowset, because a single call exchanges one payload for one result.

Each `SC_FN_*` id names its own payload field on `exascript_single_call_rep`: `json_arg` for the virtual-schema adapter, `import_specification` for `SC_FN_GENERATE_SQL_FOR_IMPORT_SPEC`, and `export_specification` for `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC`. The two specification messages are protobuf, which does not cross the `.so` boundary, so the dispatcher serializes them to the JSON shape `sdk/udf-sdk` pins and passes that as `json_spec`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: An import or export spec call delivers the serialized specification to the hook

* *GIVEN* an `MT_CALL` naming `SC_FN_GENERATE_SQL_FOR_IMPORT_SPEC` or `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC` and carrying the matching specification message
* *WHEN* the dispatcher routes it to the vtable hook
* *THEN* it MUST serialize that specification message to the JSON shape `sdk/udf-sdk` pins and pass the result as `json_spec`
* *AND* it MUST NOT pass the `json_arg` field, which carries the virtual-schema adapter payload
* *AND* a call whose matching specification message is absent MUST close the session with a prefixed `F-UDF-CL-RUST-####` error naming the missing field, rather than invoking the hook with an empty payload
* *AND* the dispatcher MUST NOT write the serialized specification to any log or trace output at any debug level, because `connection_information` carries the CONNECTION object's password
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Spec-generation hooks receive a SingleCallContext

* *GIVEN* a loaded UDF whose vtable implements `generate_sql_for_import_spec` or `generate_sql_for_export_spec`
* *WHEN* the single-call dispatcher routes the matching `HostEvent::SingleCall`
* *THEN* it MUST construct a `SingleCallContext` and pass a double-indirected `*mut c_void` context pointer to the hook, over the same path `virtual_schema_adapter_call` uses
* *AND* the hook MUST read `script_schema()`, `node_count()`, and the remaining handshake accessors at their live `MT_META` values, not at the `UdfContext` trait defaults
* *AND* `ctx.connection(name)` MUST resolve CONNECTION credentials over an on-demand `MT_IMPORT` exchange during the call
* *AND* an error the context recorded MUST be appended to the hook's own error text in the surfaced `RuntimeError::Udf`
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Single-call hook error text is surfaced when rc != 0

* *GIVEN* a loaded UDF whose single-call hook returns a non-zero error code and writes its `UdfError` message into the result out-pointer
* *WHEN* the single-call dispatcher invokes the hook via `call_noarg_hook` or `call_ctx_arg_hook`
* *THEN* the helper MUST read the out-pointer text before freeing it and surface that text in the returned `RuntimeError::Udf`
* *AND* when the out-pointer text is non-empty the surfaced message MUST be that hook-supplied text rather than the generic `single-call hook <name> returned error code <rc>` string
* *AND* when the out-pointer is null or empty the helper MUST fall back to the generic `single-call hook <name> returned error code <rc>` message
* *AND* the helper MUST NOT leak the out-pointer buffer on the error path
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Unimplemented single-call hook replies MT_UNDEFINED_CALL

* *GIVEN* a loaded UDF whose vtable leaves `generate_sql_for_export_spec` unimplemented
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Generate_Sql_For_Export_Spec`
* *THEN* the hook MUST return `UdfError::Unimplemented`
* *AND* the dispatcher MUST reply with `HostAction::UndefinedCall` naming the SDK hook an author would implement (`generate_sql_for_export_spec`) rather than the protobuf enum variant (`SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC`), falling back to the protobuf variant name only for the `SC_FN_NIL` sentinel, which names no SDK hook
* *AND* it MUST accept the DB's `MT_UNDEFINED_CALL` echo as the acknowledgement and close the run with `MT_DONE`, leaving the DB to raise its own "function not implemented" error
* *AND* an acknowledgement that does not echo the message just sent MUST close the session with an error rather than be accepted
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: IMPORT FROM SCRIPT runs the generated SQL against its worker UDF

* *GIVEN* a live Exasol database with a registered Rust SLC and the `import-export-spec` fixture registered as the scripts `IMPORT_SPEC_GEN` and `IMPORT_WORKER`
* *WHEN* a client runs `IMPORT INTO (<column list>) FROM SCRIPT IMPORT_SPEC_GEN AT <connection> WITH <parameters>`
* *THEN* the database MUST accept the statement and insert the rows the generated `SELECT` produced
* *AND* the inserted rows MUST report the connection name and every `WITH` parameter the hook read out of `json_spec`
* *AND* they MUST report the `is_subselect` flag and the names of the columns the `IMPORT INTO (...)` list declared
* *AND* they MUST report the column count and each declared column name and type that `IMPORT_WORKER` read through `ctx.input_column_count()` and `ctx.input_column(idx)` at runtime, so the variadic worker's own schema discovery is asserted live
* *AND* the generated SQL MUST qualify the worker script with `ctx.script_schema()`, so the statement only succeeds when the hook ran against a live context
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: EXPORT INTO SCRIPT surfaces the specification its hook observed

* *GIVEN* a live Exasol database with a registered Rust SLC, the `import-export-spec` fixture registered as the scripts `EXPORT_SPEC_GEN` and `EXPORT_WORKER`, and a source table of known column names
* *WHEN* a client runs `EXPORT <table> INTO SCRIPT EXPORT_SPEC_GEN AT <connection> WITH <parameters> TRUNCATE`
* *THEN* the database MUST run the generated `SELECT`, whose worker reports the observed specification over the UDF error channel
* *AND* the surfaced error text MUST carry the connection name, every `WITH` parameter, and the fully qualified source column names the database supplied
* *AND* it MUST report `has_truncate` as true and `has_replace` as false
* *AND* the error MUST arrive over the UDF error path with its `F-UDF-CL-RUST-` prefix, distinguishing a hook that ran from the database's own "function not implemented" diagnostic for an unwired slot
<!-- /DELTA:NEW -->
