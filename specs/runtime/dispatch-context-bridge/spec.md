# Feature: dispatch-context-bridge

Specifies `HostContextBridge`, the adapter that turns the host-internal `UdfMeta` and rowset codec into the `&dyn UdfContext` a scalar or set UDF sees: input row materialization, handshake identity/origin metadata, and output-row validation. The run and dispatch mechanics this bridge is threaded into are specified separately in `runtime/dispatch-run-loop`.

## Background

The `HostContextBridge` adapts the host-internal `UdfMeta` and rowset codec into the `&dyn UdfContext` the UDF sees, threading handshake metadata (memory limit and the `exascript_info` identity/origin fields) and declared input/output columns in at construction so the bridge can override the SDK's defaulted accessors with live values.

## Scenarios

### Scenario: Bridge materializes input rows into typed accessors

* *GIVEN* a `HostContextBridge` over a fake transport delivering one input batch of mixed column types, where the protobuf `ExascriptTableData` lays out values row-major within each type block (non-null cells only)
* *WHEN* a UDF calls `next` then the typed accessors
* *THEN* `next` MUST return `true` while rows remain and `false` when input is exhausted
* *AND* each typed accessor MUST return the correct value for the current row by advancing per-type cursors only on non-null cells — a NULL cell MUST NOT consume a slot in its type block
* *AND* a NULL cell MUST be returned as `Value::Null`

### Scenario: Bridge surfaces handshake identity and origin metadata to the UDF

* *GIVEN* a `HostContextBridge` constructed from a `UdfMeta` whose `exascript_info`-derived fields (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`) carry live values
* *WHEN* a UDF calls the corresponding `UdfContext` handshake accessors
* *THEN* the bridge MUST override each defaulted accessor to return the exact value carried on the originating `UdfMeta` field, performing no rescaling or reinterpretation
* *AND* the bridge MUST return the optional accessors (`current_user`, `current_schema`, `scope_user`) as `Some(value)` when the proto field was present and `None` when it was absent
* *AND* the bridge MUST source every value from `UdfMeta` threaded in at construction time, not from any per-call protocol exchange

### Scenario: Bridge validates every output row and surfaces the column metadata

* *GIVEN* a `HostContextBridge` constructed from a `UdfMeta` whose declared input and output columns carry live values
* *WHEN* a UDF produces an output row, through `emit` or through the framework's `set_return`, or reads its own column metadata
* *THEN* the bridge MUST reject a row the declared output columns cannot carry with `UdfError::Type`, before the row is buffered, so no part of it reaches the wire
* *AND* the rejection MUST close the session through the UDF-error path with the `F-UDF-CL-RUST-` prefixed message naming the offending column
* *AND* the bridge MUST override the defaulted column accessors to return the declared metadata the handshake supplied, for both the input and the output side
* *AND* a batch-emitted row MUST be validated against the same declared columns, once per batch before any row of it is materialised
