# Feature: dispatch-run-loop

Orchestrates driving the scalar/set run loop over the wire protocol — covering iteration-shape dispatch, bridge row materialisation, context-contract enforcement, UDF error propagation, and connect-back availability. The `EmitBuffer`/`InputRowSet` rowset codec this loop drives (output packing, flush-threshold accounting, and any promoted fast-path formatter/parser) is specified separately in `runtime/rowset-codec`; the opt-in Arrow batch-emit path is specified separately in `runtime/emit-arrow-batch`. Loader validation and artifact resolution are specified separately in `runtime/dispatch-loader`. Single-call dispatch is specified separately in `runtime/dispatch-single-call`. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

The runtime drives dispatch via the pure protocol state machine after a `.so` has been loaded. The `HostContextBridge` adapts the host-internal `UdfMeta` and rowset codec into the `&dyn UdfContext` the UDF sees, threading handshake metadata (memory limit and the `exascript_info` identity/origin fields) in at construction so the bridge can override the SDK's defaulted accessors with live values.

The dispatcher MUST branch on the two `UdfMeta` iteration axes. The input axis (`input_iter`: `ExactlyOnce` = scalar, `Multiple` = set) selects who drives the input loop: for scalar the framework owns the per-row loop and invokes `run()` once per input row; for set the UDF drives its own loop via `ctx.next()` and `run()` is invoked once per input group. The output axis (`output_iter`: `ExactlyOnce` = RETURNS, `Multiple` = EMITS) selects the emit contract. The contracts match the reference Exasol containers' rejection semantics; shape is a runtime property (from the handshake metadata), not a Rust compile-time property, so enforcement is at runtime and surfaced through the `F-UDF-CL-RUST-` error-close path.

RETURNS output uses a value-return channel: the UDF function returns its value (`Some(v)` → one row, `None` → SQL NULL), the framework records it through `UdfContext::set_return` and emits the single row, and author-called `ctx.emit()` is banned in RETURNS context. EMITS output is unchanged — the UDF produces rows via `ctx.emit()`. The compiled output shape (from the `.so` vtable marker) is validated against `meta.output_iter` so a mismatch is a clear error rather than UB. The SDK exposes no `reset()` method, so the reference's "`reset()` banned in scalar" rule has no SDK surface to gate.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Emit buffer and wire closures are session-scoped and reset per group

* *GIVEN* a run phase that processes more than one input group over one loaded UDF, where a SET/EMITS session of many small groups pays at least four round trips per group
* *WHEN* the runtime opens each group
* *THEN* the runtime MUST construct the `EmitBuffer`, the shared `Protocol` cell, the emit flusher closure and the batch fetcher closure once per session and reuse them across every group, instead of allocating a buffer, a cell and three boxed closures per group
* *AND* at each group boundary it MUST reset the buffer through the capacity-retaining reset, so a session of many small groups pays the row-vector allocation once
* *AND* on the first `MT_NEXT` of a SET group it MUST reserve emit-buffer row capacity from that batch's `rows_in_group`
* *AND* it MUST cap that reservation at a constant row ceiling
* *AND* the reuse MUST NOT change the existing per-group flush contract: every row of a group's output MUST still reach the wire before that group's `MT_DONE`, and no buffered row MUST carry across a group boundary
<!-- /DELTA:NEW -->
</content>
