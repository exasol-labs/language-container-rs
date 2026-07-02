# Feature: dispatch-single-call

Handles the single-call dispatch path — routing `SC_FN_*` function IDs from `MT_META` to the matching vtable hooks, replying with `MT_RETURN` or `MT_UNDEFINED_CALL`, and validating annotated UDF schemas against database metadata at load time. The scalar/set run loop is specified separately in `runtime/dispatch-run-loop`.

## Background

v2 adds single-call dispatch routing `SC_FN_*` to vtable hooks and load-time validation of typed `#[exasol_udf(input(...), emits(...))]` schemas against the database metadata. ABI v3 changes the `virtual_schema_adapter_call` dispatch to pass a `SingleCallContext` through the double-indirected `*mut c_void` ABI, so VS adapters can resolve CONNECTION credentials and open connect-back sessions mid single-call. The `SingleCallContext` also carries the `exascript_info` handshake metadata threaded in from `MT_META`, so a virtual-schema adapter sees the same live identity/origin/topology values the scalar/set run loop's `HostContextBridge` surfaces.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Adapter single-call context surfaces live handshake metadata

* *GIVEN* a single-call session whose `MT_META` handshake carried live `exascript_info` fields (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `maximal_memory_limit`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`)
* *WHEN* the dispatcher constructs the `SingleCallContext` for a `Sc_Fn_Virtual_Schema_Adapter_Call` and the adapter hook queries the `UdfContext` handshake accessors (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `memory_limit`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`)
* *THEN* the context MUST return the exact value carried on the originating handshake field for every accessor, performing no rescaling or reinterpretation, rather than the `UdfContext` trait's neutral default (`0` / empty / `None`)
* *AND* the context MUST return the optional accessors (`current_user`, `current_schema`, `scope_user`) as `Some(value)` when the proto field was present and `None` when it was absent
* *AND* the context MUST source every value from the handshake metadata threaded in at construction time, not from any per-call protocol exchange
* *AND* none of the handshake accessors MAY be gated behind the `connect-back` feature, because handshake metadata is plain DB-supplied context rather than a connect-back capability
<!-- /DELTA:NEW -->
