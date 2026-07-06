# Feature: dispatch-run-loop

Orchestrates driving the scalar/set run loop over the wire protocol — covering bridge row materialisation, UDF error propagation, and connect-back availability. The `EmitBuffer`/`InputRowSet` rowset codec this loop drives (output packing, flush-threshold accounting, the Arrow batch-emit path, and any promoted fast-path formatter/parser) is specified separately in `runtime/rowset-codec`. Loader validation and artifact resolution are specified separately in `runtime/dispatch-loader`. Single-call dispatch is specified separately in `runtime/dispatch-single-call`. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

The runtime drives dispatch via the pure protocol state machine after a `.so` has been loaded. The `HostContextBridge` adapts the host-internal `UdfMeta` and rowset codec into the `&dyn UdfContext` the UDF sees, threading handshake metadata (memory limit and the `exascript_info` identity/origin fields) in at construction so the bridge can override the SDK's defaulted accessors with live values.

## Scenarios

### Scenario: Bridge materializes input rows into typed accessors

* *GIVEN* a `HostContextBridge` over a fake transport delivering one input batch of mixed column types, where the protobuf `ExascriptTableData` lays out values row-major within each type block (non-null cells only)
* *WHEN* a UDF calls `next` then the typed accessors
* *THEN* `next` MUST return `true` while rows remain and `false` when input is exhausted
* *AND* each typed accessor MUST return the correct value for the current row by advancing per-type cursors only on non-null cells — a NULL cell MUST NOT consume a slot in its type block
* *AND* a NULL cell MUST be returned as `Value::Null`

### Scenario: Scalar dispatch runs the UDF and emits one batch

* *GIVEN* a loaded scalar UDF and a `HostContextBridge` with `iter_type = ExactlyOnce`
* *WHEN* the runtime sends `MT_RUN` and calls the vtable `run`
* *THEN* the bridge `next`/`emit` calls MUST drive `MT_NEXT`/`MT_EMIT` exchanges against the transport
* *AND* on `run` return the runtime MUST send `MT_DONE`

### Scenario: Set/EMITS dispatch emits multiple rows across batches

* *GIVEN* a loaded set UDF and a `HostContextBridge` with a wire flusher closure that serializes the `EmitBuffer` to an `ExascriptTableData`, sends `MT_EMIT`, awaits the emit ack, and clears the buffer
* *WHEN* the UDF iterates all input rows and emits a filtered subset
* *THEN* the bridge MUST accumulate emitted rows in the `EmitBuffer` and MUST trigger a mid-run `MT_EMIT` flush as soon as the buffer's running byte estimate reaches the `EMIT_BUFFER_LIMIT_BYTES` threshold of `4_000_000` bytes, rather than sending one frame per row or buffering an unbounded batch
* *AND* after the UDF's `run` method returns, the dispatch loop MUST flush any remaining buffered rows in a final `MT_EMIT` even when the byte estimate is below the threshold, so no emitted row is lost
* *AND* the total emitted row count across all flushes MUST equal the number of `emit` calls the UDF made

### Scenario: UDF error closes the session with a prefixed message

* *GIVEN* a loaded UDF whose `run` returns a non-zero error code
* *WHEN* the runtime observes the failure
* *THEN* it MUST serialize the error message into the protocol close path with the `F-UDF-CL-RUST-` prefix
* *AND* it MUST call the vtable `destroy` and drop the `Library` before returning failure

### Scenario: Dispatch reads UDF error text from the run out-pointer

* *GIVEN* `run_batch` calling the vtable `run` with the context pointer and an `error_out` out-pointer initialized to null
* *WHEN* the UDF run shim returns a non-zero code and has written a heap-allocated C string to `*error_out`
* *THEN* dispatch MUST read the C string from the out-pointer when it is non-null and take ownership so the allocation is freed exactly once, using `libc::free` consistent with the `malloc`/`libc::free` C-allocator convention
* *AND* dispatch MUST incorporate the recovered text into the `RuntimeError::Udf` message it returns, so the DB error-close path surfaces the UDF-supplied error text rather than only the generic error code
* *AND* dispatch MUST NOT rely on `take_last_error` for this path, leaving the connect-back `last_error` channel unchanged

### Scenario: Connect-back is available identically in scalar and set dispatch

* *GIVEN* a runtime driving either a scalar UDF (`iter_type = ExactlyOnce`) or a set UDF (`iter_type = Multiple`) with the connect-back feature enabled
* *WHEN* the UDF calls `ctx.connection()` or `ctx.connect_back()` during its `run` method
* *THEN* the runtime MUST handle the connect-back MT_IMPORT exchange and session open identically for both `iter_type` values — there MUST be no scalar-specific restriction, guard, or branch that prevents connect-back in the scalar path
* *AND* the ZMQ socket MUST be idle during `run` in both cases (blocked awaiting UDF function return), making the MT_IMPORT exchange safe in both scalar and set dispatch
* *AND* `std::process::exit(0)` in `main()` MUST flush the connect-back Tokio runtime in both scalar and set execution paths, preventing the 10 s join delay and the resulting Part:40 SIGABRT

### Scenario: Bridge surfaces handshake identity and origin metadata to the UDF

* *GIVEN* a `HostContextBridge` constructed from a `UdfMeta` whose `exascript_info`-derived fields (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`) carry live values
* *WHEN* a UDF calls the corresponding `UdfContext` handshake accessors
* *THEN* the bridge MUST override each defaulted accessor to return the exact value carried on the originating `UdfMeta` field, performing no rescaling or reinterpretation
* *AND* the bridge MUST return the optional accessors (`current_user`, `current_schema`, `scope_user`) as `Some(value)` when the proto field was present and `None` when it was absent
* *AND* the bridge MUST source every value from `UdfMeta` threaded in at construction time, not from any per-call protocol exchange
