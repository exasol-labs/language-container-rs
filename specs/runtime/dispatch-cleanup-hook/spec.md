# Feature: dispatch-cleanup-hook

Specifies the UDF cleanup hook: when the runtime invokes it, what context it receives, and how its outcome interacts with `MT_FINISHED` and the error-close path. The hook runs at session end for both dispatch shapes the runtime drives — the scalar/set run loop (`runtime/dispatch-run-loop`) and single-call dispatch (`runtime/dispatch-single-call`) — under the one set of rules this feature specifies.

## Background

A UDF process runs one session, spanning zero or more input groups (run loop) or zero or more single calls. When the author registered a `cleanup` hook, the runtime invokes it exactly once at session end, after the DB sends `MT_CLEANUP` and before the runtime sends any further message. The hook observes the state the UDF accumulated across every group or call that process ran.

The hook receives a `CleanupContext`, a `UdfContext` implementation distinct from `HostContextBridge` (run loop) and `SingleCallContext` (single-call). `script_name()`, `node_id()`, and the other handshake accessors return the live `MT_META` values. `ctx.next()`, `ctx.get()`, and `ctx.emit()` return `Err(UdfError)`, because no input or output remains. `ctx.connect_back(conn)` behaves as in `SingleCallContext`, so a `ConnectionObject` resolved during `run` opens a connect-back session, because that login travels over TCP and sends nothing on the control channel. `ctx.connection(name)` refuses every lookup without sending `MT_IMPORT`, because after `MT_CLEANUP` the database accepts no message besides `MT_FINISHED` and `MT_CLOSE`.

On `Ok(())` the runtime sends `MT_FINISHED`. On `Err(UdfError)` the runtime closes the session through the error-close path with the `F-UDF-CL-RUST-` prefix instead, carrying the text the hook wrote to its error out-pointer. When dispatch itself already ended in an error — `run` returned an error, the DB sent `MT_CLOSE`, or a single-call hook failed — the runtime still invokes the cleanup hook before its own error close, and a cleanup failure appends the cleanup error text after the original error text in the one close message. Validation failure before any group or call ran (compiled output shape or annotated schema mismatch) closes the session with the validation error alone and skips the cleanup hook, because no VM construction succeeded, matching the reference client's behavior.

## Scenarios

### Scenario: The cleanup hook runs once after the last group, before MT_FINISHED

* *GIVEN* a loaded UDF that registered a cleanup hook and whose process runs zero or more input groups
* *WHEN* the DB ends the session with `MT_CLEANUP`
* *THEN* the runtime MUST invoke the cleanup hook exactly once, after the last group's `MT_DONE` exchange (or after the first `MT_RUN` when no group ran) and before it sends any further message
* *AND* the hook MUST observe the state the UDF accumulated over every group that process ran, because one UDF process runs one session
* *AND* when the hook returns `Ok(())` the runtime MUST then send `MT_FINISHED`, and the statement MUST succeed on a live database

### Scenario: A cleanup hook error fails the statement instead of MT_FINISHED

* *GIVEN* a loaded UDF whose cleanup hook returns `Err(UdfError)`
* *WHEN* the runtime invokes the hook after the DB sent `MT_CLEANUP`
* *THEN* the runtime MUST NOT send `MT_FINISHED`
* *AND* it MUST close the session through the error-close path with the `F-UDF-CL-RUST-` prefix, carrying the text the hook wrote to its error out-pointer
* *AND* the statement MUST fail on a live database with that text

### Scenario: An error that ends dispatch still runs the cleanup hook and reports both errors

* *GIVEN* a loaded UDF that registered a cleanup hook
* *WHEN* dispatch ends because `run` returned an error or because the DB answered a request with `MT_CLOSE`
* *THEN* the runtime MUST invoke the cleanup hook before it sends its own error close
* *AND* when the hook also fails, the single error-close message MUST carry the original error text followed by the cleanup error text
* *AND* the runtime MUST read the cleanup error text from the hook's error out-pointer and free it exactly once, following the `malloc`/`libc::free` convention of the `run` out-pointer

### Scenario: Validation failure before dispatch skips the cleanup hook

* *GIVEN* a loaded UDF that registered a cleanup hook
* *WHEN* its compiled output shape or its annotated schema fails validation against the handshake metadata
* *THEN* the runtime MUST close the session with the validation error alone
* *AND* it MUST NOT invoke the cleanup hook, because no group or call ran, as the reference client skips cleanup when VM construction fails

### Scenario: The cleanup hook receives a CleanupContext with handshake metadata and connect-back

* *GIVEN* a UDF whose cleanup hook reads its `&mut dyn UdfContext`
* *WHEN* the runtime invokes the hook at session end, after the run loop or after the single-call loop
* *THEN* the runtime MUST pass a `CleanupContext`, a `UdfContext` implementation distinct from `HostContextBridge` and `SingleCallContext`, through the double-indirected context pointer every hook receives
* *AND* `script_name()`, `node_id()`, and the other handshake accessors MUST return the live `MT_META` values, delegating to the handshake metadata exactly as `SingleCallContext` does
* *AND* `ctx.next()`, `ctx.get()`, and `ctx.emit()` MUST return `Err(UdfError)`, as in `SingleCallContext`, because no input or output remains
* *AND* `ctx.connect_back(conn)` MUST behave as in `SingleCallContext`, so a `ConnectionObject` the UDF resolved during `run` opens a connect-back session, because that login travels over TCP and sends nothing on the control channel

### Scenario: The CleanupContext refuses CONNECTION lookups without sending MT_IMPORT

* *GIVEN* a cleanup hook that calls `ctx.connection(name)` on its `CleanupContext`
* *WHEN* the runtime invokes the hook at session end
* *THEN* `ctx.connection(name)` MUST return `Err(UdfError::ConnectBack)` immediately for every name, and the runtime MUST NOT send `MT_IMPORT`, because after `MT_CLEANUP` the database accepts no message besides `MT_FINISHED` and `MT_CLOSE`
* *AND* the error text MUST state that CONNECTION lookups are unavailable during cleanup
* *AND* the error text MUST tell the author to resolve the `ConnectionObject` during `run()` and keep it for cleanup

### Scenario: The cleanup hook runs after the single-call loop

* *GIVEN* a single-call session whose UDF registered a cleanup hook
* *WHEN* the DB ends the session with `MT_CLEANUP` after the last call
* *THEN* the runtime MUST invoke the cleanup hook once, before `MT_FINISHED`, under the context and error rules this feature specifies for the run loop
* *AND* the hook MUST receive a `CleanupContext`, not the `SingleCallContext` the session's calls received, so its `ctx.connection(name)` returns the cleanup refusal without sending `MT_IMPORT`
* *AND* an error that ends the single-call loop, such as a missing specification message, MUST still run the cleanup hook before the error close, with the cleanup error text after the original error text
* *AND* on a live database, an `EXPORT ... INTO SCRIPT` statement whose callback script's cleanup hook fails MUST fail with the `F-UDF-CL-RUST-` prefix and the text the cleanup hook returned
