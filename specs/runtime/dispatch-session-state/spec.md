# Feature: dispatch-session-state

Specifies the session-scoped state the dispatcher reuses across an input group: the `EmitBuffer`, the shared `Protocol` cell, and the emit/batch-fetch closures, plus the handshake identity and origin metadata the `HostContextBridge` surfaces to the UDF for the life of the session. The per-group run-loop dispatch (iteration-shape dispatch, bridge row materialisation, error propagation, connect-back availability) is specified separately in `runtime/dispatch-run-loop`.

## Background

A run phase spans one loaded UDF across many input groups. Reallocating the `EmitBuffer`, the shared `Protocol` cell, and the three boxed closures (emit flusher, batch fetcher) on every group boundary wastes an allocation for a SET/EMITS session that already pays at least four round trips per group. These constructs MUST instead be built once per session and reused, with the buffer reset (capacity retained) at each group boundary.

The `HostContextBridge` also threads handshake metadata (the `exascript_info` identity/origin fields, plus the memory limit) in at construction, once per session, so the bridge can override the SDK's defaulted accessors with live values for the whole session rather than re-deriving them per group.

## Scenarios

### Scenario: Emit buffer and wire closures are session-scoped and reset per group

* *GIVEN* a run phase that processes more than one input group over one loaded UDF, where a SET/EMITS session of many small groups pays at least four round trips per group
* *WHEN* the runtime opens each group
* *THEN* the runtime MUST construct the `EmitBuffer`, the shared `Protocol` cell, the emit flusher closure and the batch fetcher closure once per session and reuse them across every group, instead of allocating a buffer, a cell and three boxed closures per group
* *AND* at each group boundary it MUST reset the buffer through the capacity-retaining reset, so a session of many small groups pays the row-vector allocation once
* *AND* on the first `MT_NEXT` of a SET group it MUST reserve emit-buffer row capacity from that batch's `rows_in_group`
* *AND* it MUST cap that reservation at a constant row ceiling
* *AND* the reuse MUST NOT change the existing per-group flush contract: every row of a group's output MUST still reach the wire before that group's `MT_DONE`, and no buffered row MUST carry across a group boundary

### Scenario: Bridge surfaces handshake identity and origin metadata to the UDF

* *GIVEN* a `HostContextBridge` constructed from a `UdfMeta` whose `exascript_info`-derived fields (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`) carry live values
* *WHEN* a UDF calls the corresponding `UdfContext` handshake accessors
* *THEN* the bridge MUST override each defaulted accessor to return the exact value carried on the originating `UdfMeta` field, performing no rescaling or reinterpretation
* *AND* the bridge MUST return the optional accessors (`current_user`, `current_schema`, `scope_user`) as `Some(value)` when the proto field was present and `None` when it was absent
* *AND* the bridge MUST source every value from `UdfMeta` threaded in at construction time, not from any per-call protocol exchange
