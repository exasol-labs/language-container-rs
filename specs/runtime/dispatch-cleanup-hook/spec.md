# Feature: dispatch-cleanup-hook

Specifies the UDF cleanup hook: when the runtime invokes it, what context it receives, and how its outcome interacts with `MT_FINISHED` and the error-close path. The same rules apply after the scalar/set run loop (`runtime/dispatch-run-loop`) and after single-call dispatch (`runtime/dispatch-single-call`).

## Background

A UDF process runs one session, spanning zero or more input groups or single calls. When the author registered a `cleanup` hook, the runtime invokes it exactly once at session end, after the DB sends `MT_CLEANUP` and before the runtime sends any further message, so the hook observes the state the UDF accumulated across that process.

## Scenarios

### Scenario: The cleanup hook runs once after the last group, before MT_FINISHED

* *GIVEN* a loaded UDF that registered a cleanup hook and whose process runs zero or more input groups
* *WHEN* the DB ends the session with `MT_CLEANUP`
* *THEN* the runtime MUST invoke the cleanup hook exactly once, after the last group's `MT_DONE` exchange (or after the first `MT_RUN` when no group ran) and before it sends any further message
* *AND* the hook MUST observe the state the UDF accumulated over every group that process ran
* *AND* when the hook returns `Ok(())` the runtime MUST then send `MT_FINISHED`, and the statement MUST succeed on a live database

### Scenario: A cleanup hook error fails the statement instead of MT_FINISHED

* *GIVEN* a loaded UDF whose cleanup hook returns `Err(UdfError)`
* *WHEN* the runtime invokes the hook after the DB sent `MT_CLEANUP`
* *THEN* the runtime MUST NOT send `MT_FINISHED`
* *AND* it MUST close the session through the error-close path with the `F-UDF-CL-RUST-` prefix, carrying the hook's error text
* *AND* the statement MUST fail on a live database with that text, including an `EXPORT ... INTO SCRIPT` statement whose callback script's cleanup hook fails

### Scenario: An error that ends dispatch still runs the cleanup hook and reports both errors

* *GIVEN* a loaded UDF that registered a cleanup hook
* *WHEN* dispatch ends because `run` or a single-call hook returned an error, or because the DB answered a request with `MT_CLOSE`
* *THEN* the runtime MUST invoke the cleanup hook before it sends its own error close
* *AND* when the hook also fails, the single error-close message MUST carry the original error text followed by the cleanup error text, and a live database MUST relay both

### Scenario: Validation failure before dispatch skips the cleanup hook

* *GIVEN* a loaded UDF that registered a cleanup hook
* *WHEN* its compiled output shape or its annotated schema fails validation against the handshake metadata
* *THEN* the runtime MUST close the session with the validation error alone
* *AND* it MUST NOT invoke the cleanup hook, as the reference client skips cleanup when VM construction fails

### Scenario: The cleanup hook receives a CleanupContext that refuses CONNECTION lookups

* *GIVEN* a UDF whose cleanup hook reads its `&mut dyn UdfContext`, after the run loop or after the single-call loop
* *WHEN* the runtime invokes the hook at session end
* *THEN* the runtime MUST pass a `CleanupContext` whose handshake accessors, such as `script_name()` and `node_id()`, return the live `MT_META` values
* *AND* `ctx.next()`, `ctx.get()`, and `ctx.emit()` MUST return `Err(UdfError)`, because no input or output remains
* *AND* `ctx.connect_back(conn)` MUST open a connect-back session with a `ConnectionObject` resolved during `run`, because that login travels over TCP and sends nothing on the control channel
* *AND* `ctx.connection(name)` MUST return `Err(UdfError::ConnectBack)` without sending `MT_IMPORT`, because after `MT_CLEANUP` the database accepts only `MT_FINISHED` and `MT_CLOSE`; the error text MUST state that CONNECTION lookups are unavailable during cleanup and tell the author to resolve the connection in `run()` and keep it
