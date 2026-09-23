<!-- DELTA:CHANGED -->
# Feature: dispatch-run-loop

Orchestrates driving the scalar/set run loop over the wire protocol, covering iteration-shape dispatch, bridge row materialisation, context-contract enforcement, UDF error propagation, connect-back availability, and the session-end cleanup hook that single-call dispatch shares. The `EmitBuffer`/`InputRowSet` rowset codec this loop drives (output packing, flush-threshold accounting, and any promoted fast-path formatter/parser) is specified separately in `runtime/rowset-codec`; the opt-in Arrow batch-emit path is specified separately in `runtime/emit-arrow-batch`. Loader validation and artifact resolution are specified separately in `runtime/dispatch-loader`. Single-call dispatch is specified separately in `runtime/dispatch-single-call`. The connect-back host implementation is specified separately in `runtime/connect-back`.
<!-- /DELTA:CHANGED -->

## Background

The runtime drives dispatch via the pure protocol state machine after a `.so` has been loaded. The `HostContextBridge` adapts the host-internal `UdfMeta` and rowset codec into the `&dyn UdfContext` the UDF sees, threading handshake metadata (memory limit and the `exascript_info` identity/origin fields) in at construction so the bridge can override the SDK's defaulted accessors with live values.

The dispatcher MUST branch on the two `UdfMeta` iteration axes. The input axis (`input_iter`: `ExactlyOnce` = scalar, `Multiple` = set) selects who drives the input loop: for scalar the framework owns the per-row loop and invokes `run()` once per input row; for set the UDF drives its own loop via `ctx.next()` and `run()` is invoked once per input group. The output axis (`output_iter`: `ExactlyOnce` = RETURNS, `Multiple` = EMITS) selects the emit contract. The contracts match the reference Exasol containers' rejection semantics; shape is a runtime property (from the handshake metadata), not a Rust compile-time property, so enforcement is at runtime and surfaced through the `F-UDF-CL-RUST-` error-close path.

RETURNS output uses a value-return channel: the UDF function returns its value (`Some(v)` → one row, `None` → SQL NULL), the framework records it through `UdfContext::set_return` and emits the single row, and author-called `ctx.emit()` is banned in RETURNS context. EMITS output is unchanged — the UDF produces rows via `ctx.emit()`. The compiled output shape (from the `.so` vtable marker) is validated against `meta.output_iter` so a mismatch is a clear error rather than UB. The SDK exposes no `reset()` method, so the reference's "`reset()` banned in scalar" rule has no SDK surface to gate.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: UDF error closes the session with a prefixed message

* *GIVEN* a loaded UDF whose `run` returns a non-zero error code
* *WHEN* the runtime observes the failure
* *THEN* it MUST serialize the error message into the protocol close path with the `F-UDF-CL-RUST-` prefix
* *AND* it MUST run the UDF's cleanup hook, when the UDF registered one, before sending that close, and MUST drop the `Library` before returning failure
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: The cleanup hook runs once after the last group, before MT_FINISHED

* *GIVEN* a loaded UDF that registered a cleanup hook and whose process runs zero or more input groups
* *WHEN* the DB ends the session with `MT_CLEANUP`
* *THEN* the runtime MUST invoke the cleanup hook exactly once, after the last group's `MT_DONE` exchange (or after the first `MT_RUN` when no group ran) and before it sends any further message
* *AND* the hook MUST observe the state the UDF accumulated over every group that process ran, because one UDF process runs one session
* *AND* when the hook returns `Ok(())` the runtime MUST then send `MT_FINISHED`, and the statement MUST succeed on a live database
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A cleanup hook error fails the statement instead of MT_FINISHED

* *GIVEN* a loaded UDF whose cleanup hook returns `Err(UdfError)`
* *WHEN* the runtime invokes the hook after the DB sent `MT_CLEANUP`
* *THEN* the runtime MUST NOT send `MT_FINISHED`
* *AND* it MUST close the session through the error-close path with the `F-UDF-CL-RUST-` prefix, carrying the text the hook wrote to its error out-pointer
* *AND* the statement MUST fail on a live database with that text
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: An error that ends dispatch still runs the cleanup hook and reports both errors

* *GIVEN* a loaded UDF that registered a cleanup hook
* *WHEN* dispatch ends because `run` returned an error or because the DB answered a request with `MT_CLOSE`
* *THEN* the runtime MUST invoke the cleanup hook before it sends its own error close
* *AND* when the hook also fails, the single error-close message MUST carry the original error text followed by the cleanup error text
* *AND* the runtime MUST read the cleanup error text from the hook's error out-pointer and free it exactly once, following the `malloc`/`libc::free` convention of the `run` out-pointer
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Validation failure before dispatch skips the cleanup hook

* *GIVEN* a loaded UDF that registered a cleanup hook
* *WHEN* its compiled output shape or its annotated schema fails validation against the handshake metadata
* *THEN* the runtime MUST close the session with the validation error alone
* *AND* it MUST NOT invoke the cleanup hook, because no group or call ran, as the reference client skips cleanup when VM construction fails
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The cleanup hook receives a CleanupContext with handshake metadata and connect-back

* *GIVEN* a UDF whose cleanup hook reads its `&mut dyn UdfContext`
* *WHEN* the runtime invokes the hook at session end, after the run loop or after the single-call loop
* *THEN* the runtime MUST pass a `CleanupContext`, a `UdfContext` implementation distinct from `HostContextBridge` and `SingleCallContext`, through the double-indirected context pointer every hook receives
* *AND* `script_name()`, `node_id()`, and the other handshake accessors MUST return the live `MT_META` values, delegating to the handshake metadata exactly as `SingleCallContext` does
* *AND* `ctx.next()`, `ctx.get()`, and `ctx.emit()` MUST return `Err(UdfError)`, as in `SingleCallContext`, because no input or output remains
* *AND* `ctx.connect_back(conn)` MUST behave as in `SingleCallContext`, so a `ConnectionObject` the UDF resolved during `run` opens a connect-back session, because that login travels over TCP and sends nothing on the control channel
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The CleanupContext refuses CONNECTION lookups without sending MT_IMPORT

* *GIVEN* a cleanup hook that calls `ctx.connection(name)` on its `CleanupContext`
* *WHEN* the runtime invokes the hook at session end
* *THEN* `ctx.connection(name)` MUST return `Err(UdfError::ConnectBack)` immediately for every name, and the runtime MUST NOT send `MT_IMPORT`, because after `MT_CLEANUP` the database accepts no message besides `MT_FINISHED` and `MT_CLOSE`
* *AND* the error text MUST state that CONNECTION lookups are unavailable during cleanup
* *AND* the error text MUST tell the author to resolve the `ConnectionObject` during `run()` and keep it for cleanup
<!-- /DELTA:NEW -->
