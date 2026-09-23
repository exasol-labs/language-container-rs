# Feature: dispatch-spec-generation

Specifies the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` single-call hooks: how the dispatcher delivers a serialized `IMPORT`/`EXPORT` specification to the hook and how the generated SQL round-trips against a live database. Routing a `SC_FN_*` id to its vtable hook in general is specified separately in `runtime/dispatch-single-call`.

## Background

Each `SC_FN_GENERATE_SQL_FOR_IMPORT_SPEC` / `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC` call names its own payload field on `exascript_single_call_rep`: `import_specification` or `export_specification`. Both specification messages are protobuf, which does not cross the `.so` boundary, so the dispatcher serializes them to the JSON shape `sdk/udf-sdk` pins and passes that as `json_spec`.

The hook receives a `SingleCallContext` through the double-indirected `*mut c_void` ABI, over the same path `virtual_schema_adapter_call` uses. The context carries the `exascript_info` handshake metadata threaded in from `MT_META` and resolves CONNECTION credentials over an on-demand `MT_IMPORT` exchange, so the hook sees the same live identity, origin, and topology values the scalar/set run loop's `HostContextBridge` surfaces.

## Scenarios

### Scenario: IMPORT FROM SCRIPT runs the generated SQL against its worker UDF

* *GIVEN* a live Exasol database with a registered Rust SLC and the `import-export-spec` fixture registered as the scripts `IMPORT_SPEC_GEN` and `IMPORT_WORKER`
* *WHEN* a client runs `IMPORT INTO (<column list>) FROM SCRIPT IMPORT_SPEC_GEN AT <connection> WITH <parameters>`
* *THEN* the database MUST accept the statement and insert the rows the generated `SELECT` produced
* *AND* the inserted rows MUST report the connection name and every `WITH` parameter the hook read out of `json_spec`
* *AND* they MUST report the `is_subselect` flag and the names of the columns the `IMPORT INTO (...)` list declared
* *AND* they MUST report the column count and each declared column name and type that `IMPORT_WORKER` read through `ctx.input_column_count()` and `ctx.input_column(idx)` at runtime, so the variadic worker's own schema discovery is asserted live
* *AND* the generated SQL MUST qualify the worker script with `ctx.script_schema()`, so the statement only succeeds when the hook ran against a live context

### Scenario: EXPORT INTO SCRIPT surfaces the specification its hook observed

* *GIVEN* a live Exasol database with a registered Rust SLC, the `import-export-spec` fixture registered as the scripts `EXPORT_SPEC_GEN` and `EXPORT_WORKER`, and a source table of known column names
* *WHEN* a client runs `EXPORT <table> INTO SCRIPT EXPORT_SPEC_GEN AT <connection> WITH <parameters> TRUNCATE`
* *THEN* the database MUST run the generated `SELECT`, whose worker reports the observed specification over the UDF error channel
* *AND* the surfaced error text MUST carry the connection name, every `WITH` parameter, and the fully qualified source column names the database supplied
* *AND* it MUST report `has_truncate` as true and `has_replace` as false
* *AND* the error MUST arrive over the UDF error path with its `F-UDF-CL-RUST-` prefix, distinguishing a hook that ran from the database's own "function not implemented" diagnostic for an unwired slot

### Scenario: An import or export spec call delivers the serialized specification to the hook

* *GIVEN* an `MT_CALL` naming `SC_FN_GENERATE_SQL_FOR_IMPORT_SPEC` or `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC` and carrying the matching specification message
* *WHEN* the dispatcher routes it to the vtable hook
* *THEN* it MUST serialize that specification message to the JSON shape `sdk/udf-sdk` pins and pass the result as `json_spec`
* *AND* it MUST NOT pass the `json_arg` field, which carries the virtual-schema adapter payload
* *AND* a call whose matching specification message is absent MUST close the session with a prefixed `F-UDF-CL-RUST-####` error naming the missing field, rather than invoking the hook with an empty payload
* *AND* the dispatcher MUST NOT write the serialized specification to any log or trace output at any debug level, because `connection_information` carries the CONNECTION object's password

### Scenario: Spec-generation hooks receive a SingleCallContext

* *GIVEN* a loaded UDF whose vtable implements `generate_sql_for_import_spec` or `generate_sql_for_export_spec`
* *WHEN* the single-call dispatcher routes the matching `HostEvent::SingleCall`
* *THEN* it MUST construct a `SingleCallContext` and pass a double-indirected `*mut c_void` context pointer to the hook, over the same path `virtual_schema_adapter_call` uses
* *AND* the hook MUST read `script_schema()`, `node_count()`, and the remaining handshake accessors at their live `MT_META` values, not at the `UdfContext` trait defaults
* *AND* `ctx.connection(name)` MUST resolve CONNECTION credentials over an on-demand `MT_IMPORT` exchange during the call
* *AND* an error the context recorded MUST be appended to the hook's own error text in the surfaced `RuntimeError::Udf`
