# Feature: dispatch-single-call

Handles the single-call dispatch path — routing `SC_FN_*` function IDs from `MT_META` to the matching vtable hooks, replying with `MT_RETURN` or `MT_UNDEFINED_CALL`, and validating annotated UDF schemas against database metadata at load time. The scalar/set run loop is specified separately in `runtime/dispatch-run-loop`.

## Background

v2 adds single-call dispatch routing `SC_FN_*` to vtable hooks and load-time validation of typed `#[exasol_udf(input(...), emits(...))]` schemas against the database metadata. ABI v3 changes the `virtual_schema_adapter_call` dispatch to pass a `SingleCallContext` through the double-indirected `*mut c_void` ABI, so VS adapters can resolve CONNECTION credentials and open connect-back sessions mid single-call.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Single-call hook error text is surfaced when rc != 0

* *GIVEN* a loaded UDF whose single-call hook returns a non-zero error code and writes its `UdfError` message into the result out-pointer
* *WHEN* the single-call dispatcher invokes the hook via `call_noarg_hook`, `call_arg_hook`, or `call_ctx_arg_hook`
* *THEN* the helper MUST read the out-pointer text before freeing it and surface that text in the returned `RuntimeError::Udf`
* *AND* when the out-pointer text is non-empty the surfaced message MUST be that hook-supplied text rather than the generic `single-call hook <name> returned error code <rc>` string
* *AND* when the out-pointer is null or empty the helper MUST fall back to the generic `single-call hook <name> returned error code <rc>` message
* *AND* the helper MUST NOT leak the out-pointer buffer on the error path
<!-- /DELTA:NEW -->
