# Feature: dispatch-single-call

Handles the single-call dispatch path — routing `SC_FN_*` function IDs from `MT_META` to the matching vtable hooks, replying with `MT_RETURN` or `MT_UNDEFINED_CALL`, and validating annotated UDF schemas against database metadata at load time. The scalar/set run loop is specified separately in `runtime/dispatch-run-loop`.

<!-- DELTA:CHANGED -->
## Background

v2 adds single-call dispatch routing `SC_FN_*` to vtable hooks and load-time validation of typed `#[exasol_udf(input(...), emits(...))]` schemas against the database metadata. ABI v3 changes the `virtual_schema_adapter_call` dispatch to pass a `SingleCallContext` through the double-indirected `*mut c_void` ABI, so VS adapters can resolve CONNECTION credentials and open connect-back sessions mid single-call. The `SingleCallContext` also carries the `exascript_info` handshake metadata threaded in from `MT_META`, so a virtual-schema adapter sees the same live identity/origin/topology values the scalar/set run loop's `HostContextBridge` surfaces.

Every single-call hook that can need host state receives a `SingleCallContext` through the double-indirected `*mut c_void` ABI: `virtual_schema_adapter_call`, `generate_sql_for_import_spec`, and `generate_sql_for_export_spec`. The context carries the `exascript_info` handshake metadata threaded in from `MT_META` and resolves CONNECTION credentials over an on-demand `MT_IMPORT` exchange, so a hook sees the same live identity, origin, and topology values the scalar/set run loop's `HostContextBridge` surfaces. It reports no rowset, because a single call exchanges one payload for one result.

The cleanup hook that ends a single-call session is not a single-call hook. It runs after `MT_CLEANUP`, where the database accepts no `MT_IMPORT`, so it receives the `CleanupContext` that `runtime/dispatch-run-loop` specifies.

Each `SC_FN_*` id names its own payload field on `exascript_single_call_rep`: `json_arg` for the virtual-schema adapter, `import_specification` for `SC_FN_GENERATE_SQL_FOR_IMPORT_SPEC`, and `export_specification` for `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC`. The two specification messages are protobuf, which does not cross the `.so` boundary, so the dispatcher serializes them to the JSON shape `sdk/udf-sdk` pins and passes that as `json_spec`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: The cleanup hook runs after the single-call loop

* *GIVEN* a single-call session whose UDF registered a cleanup hook
* *WHEN* the DB ends the session with `MT_CLEANUP` after the last call
* *THEN* the runtime MUST invoke the cleanup hook once, before `MT_FINISHED`, under the context and error rules `runtime/dispatch-run-loop` specifies for the run loop
* *AND* the hook MUST receive a `CleanupContext`, not the `SingleCallContext` the session's calls received, so its `ctx.connection(name)` returns the cleanup refusal without sending `MT_IMPORT`
* *AND* an error that ends the single-call loop, such as a missing specification message, MUST still run the cleanup hook before the error close, with the cleanup error text after the original error text
* *AND* on a live database, an `EXPORT ... INTO SCRIPT` statement whose callback script's cleanup hook fails MUST fail with the `F-UDF-CL-RUST-` prefix and the text the cleanup hook returned
<!-- /DELTA:NEW -->
