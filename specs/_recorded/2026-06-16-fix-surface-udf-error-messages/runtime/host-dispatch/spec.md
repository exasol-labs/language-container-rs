# Feature: host-dispatch

Orchestrates loading a UDF `.so`, building the host-side `UdfContext` bridge, and dispatching the database execution model — scalar/set run loops and single-call functions — over the wire protocol. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

`run_batch` in `dispatch.rs` builds a `HostContextBridge`, threads it across the ABI as a double-indirected `*mut c_void`, and calls the vtable `run`. On a non-zero return it builds a `RuntimeError::Udf` message that travels back to the DB through the protocol error-close path. Previously, when a UDF returned `Err(UdfError)`, the run shim discarded the message, so dispatch had nothing to report beyond the error code (the bridge `last_error` slot is written only by connect-back failures, not by plain `Err` returns). This delta adds a UDF-error channel that does not touch the connect-back `last_error` plumbing: dispatch passes an out-pointer to the vtable `run`, and the shim fills it with the UDF's error text.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Dispatch reads UDF error text from the run out-pointer

* *GIVEN* `run_batch` calling the vtable `run` with the context pointer and an `error_out` out-pointer initialized to null
* *WHEN* the UDF run shim returns a non-zero code and has written a heap-allocated C string to `*error_out`
* *THEN* dispatch MUST read the C string from the out-pointer when it is non-null and take ownership so the allocation is freed exactly once
* *AND* dispatch MUST incorporate the recovered text into the `RuntimeError::Udf` message it returns, so the DB error-close path surfaces the UDF-supplied error text rather than only the generic error code
* *AND* dispatch MUST NOT rely on `take_last_error` for this path, leaving the connect-back `last_error` channel unchanged
<!-- /DELTA:NEW -->
