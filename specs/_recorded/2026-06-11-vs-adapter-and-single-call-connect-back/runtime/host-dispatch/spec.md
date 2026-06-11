# Feature: host-dispatch

Orchestrates loading a UDF `.so`, building the host-side `UdfContext` bridge, and dispatching the database execution model — scalar/set run loops and single-call functions — over the wire protocol. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

The runtime loads a precompiled `.so` (Option A), gating on ABI version and SDK fingerprint, then drives dispatch via the pure protocol state machine. JIT compilation remains unsupported. ABI v3 changes the `virtual_schema_adapter_call` dispatch to pass a `SingleCallContext` through the double-indirected `*mut c_void` ABI, so VS adapters can resolve CONNECTION credentials and open connect-back sessions mid single-call. The rowset codec (`InputRowSet`/`EmitBuffer`) switches from column-major packing with NULL placeholders to row-major packing where NULL cells occupy no slot in their type block, and output values are packed by declared column `ExaType` rather than by runtime `Value` variant.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Virtual-schema adapter call is dispatched to the adapter hook

* *GIVEN* a loaded UDF whose vtable implements `virtual_schema_adapter_call`
* *WHEN* the single-call dispatcher receives a `HostEvent::SingleCall` with `Sc_Fn_Virtual_Schema_Adapter_Call` carrying a request string
* *THEN* the dispatcher MUST construct a `SingleCallContext` and pass a double-indirected `*mut c_void` context pointer to the hook
* *AND* it MUST invoke the `virtual_schema_adapter_call` hook with `(ctx_ptr, json_payload, result)` via `call_ctx_arg_hook`
* *AND* on success it MUST reply with `HostAction::SingleCallReturn` carrying the adapter response string
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Bridge materializes input rows into typed accessors

* *GIVEN* a `HostContextBridge` over a fake transport delivering one input batch of mixed column types, where the protobuf `ExascriptTableData` lays out values row-major within each type block (non-null cells only)
* *WHEN* a UDF calls `next` then the typed accessors
* *THEN* `next` MUST return `true` while rows remain and `false` when input is exhausted
* *AND* each typed accessor MUST return the correct value for the current row by advancing per-type cursors only on non-null cells — a NULL cell MUST NOT consume a slot in its type block
* *AND* a NULL cell MUST be returned as `Value::Null`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: EmitBuffer packs output values row-major by declared column type

* *GIVEN* an `EmitBuffer` holding rows where a column's declared `ExaType` differs from the runtime `Value` variant (e.g. `ExaType::Numeric` with `Value::Int64`)
* *WHEN* `EmitBuffer::to_proto` is called with the declared column metadata
* *THEN* each value MUST be packed into the type block dictated by the declared `ExaType`, not by the `Value` variant — a `Value::Int64` in a `Numeric` column MUST be stringified and written to the string block
* *AND* values for successive columns of the same type within the same row MUST appear contiguously in row-major order within their type block
* *AND* a NULL cell MUST NOT occupy any slot in its type block — only the null-bitmap is updated

### Scenario: InputRowSet decodes row-major type blocks correctly

* *GIVEN* a `ExascriptTableData` whose type blocks are populated row-major by `EmitBuffer::to_proto` (non-null cells only, per declared column type)
* *WHEN* `InputRowSet::from_proto` decodes the table
* *THEN* it MUST reconstruct the original row/column values by advancing per-type cursors only for non-null cells
* *AND* the decoded rows MUST match the values that were emitted, preserving column types according to the declared metadata
<!-- /DELTA:NEW -->
