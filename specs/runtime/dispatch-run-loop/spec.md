# Feature: dispatch-run-loop

Orchestrates driving the scalar/set run loop over the wire protocol — covering bridge row materialisation, emit buffering, UDF error propagation, and connect-back availability. Loader validation and artifact resolution are specified separately in `runtime/dispatch-loader`. Single-call dispatch is specified separately in `runtime/dispatch-single-call`. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

The runtime drives dispatch via the pure protocol state machine after a `.so` has been loaded. JIT compilation remains unsupported in v2 (`compiler.rs` returns `UnsupportedFeature`). The rowset codec (`InputRowSet`/`EmitBuffer`) switches from column-major packing with NULL placeholders to row-major packing where NULL cells occupy no slot in their type block, and output values are packed by declared column `ExaType` rather than by runtime `Value` variant.

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

### Scenario: Dispatch reads UDF error text from the run out-pointer

* *GIVEN* `run_batch` calling the vtable `run` with the context pointer and an `error_out` out-pointer initialized to null
* *WHEN* the UDF run shim returns a non-zero code and has written a heap-allocated C string to `*error_out`
* *THEN* dispatch MUST read the C string from the out-pointer when it is non-null and take ownership so the allocation is freed exactly once, using `libc::free` consistent with the `malloc`/`libc::free` C-allocator convention
* *AND* dispatch MUST incorporate the recovered text into the `RuntimeError::Udf` message it returns, so the DB error-close path surfaces the UDF-supplied error text rather than only the generic error code
* *AND* dispatch MUST NOT rely on `take_last_error` for this path, leaving the connect-back `last_error` channel unchanged

### Scenario: A single emitted row larger than the flush threshold is sent on its own

* *GIVEN* a loaded set UDF whose single emitted row carries a value whose serialized size alone exceeds `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000` bytes)
* *WHEN* the UDF calls `emit` once with that oversized row
* *THEN* the bridge MUST push the whole row into the `EmitBuffer` as one unit and MUST NOT split a single row across `MT_EMIT` frames, because the wire protocol packs rows atomically
* *AND* the bridge MUST then observe that the buffer's byte estimate crosses the threshold and flush the single-row buffer in one `MT_EMIT`, accepting that the frame exceeds the nominal 4,000,000-byte target rather than dropping or truncating the row
* *AND* the only hard ceiling that remains MUST be the protocol's 2 GB per-value limit, which the runtime does not attempt to circumvent

### Scenario: EmitBuffer tracks a running byte estimate and reports when to flush

* *GIVEN* a fresh `EmitBuffer`
* *WHEN* rows are appended via `push`
* *THEN* `push` MUST increase a `byte_estimate` field by an approximation of the wire size of the pushed values (summing per-value byte costs), and `should_flush` MUST return true exactly when `byte_estimate` is greater than or equal to `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`)
* *AND* `clear` MUST reset both the row vector and the `byte_estimate` to zero so a flushed buffer starts a fresh accounting cycle
* *AND* the byte estimate MUST be a monotonic non-negative running total computed without re-serializing the whole buffer on every `push`, so emit cost stays linear in the number of rows

### Scenario: Connect-back is available identically in scalar and set dispatch

* *GIVEN* a runtime driving either a scalar UDF (`iter_type = ExactlyOnce`) or a set UDF (`iter_type = Multiple`) with the connect-back feature enabled
* *WHEN* the UDF calls `ctx.connection()` or `ctx.connect_back()` during its `run` method
* *THEN* the runtime MUST handle the connect-back MT_IMPORT exchange and session open identically for both `iter_type` values — there MUST be no scalar-specific restriction, guard, or branch that prevents connect-back in the scalar path
* *AND* the ZMQ socket MUST be idle during `run` in both cases (blocked awaiting UDF function return), making the MT_IMPORT exchange safe in both scalar and set dispatch
* *AND* `std::process::exit(0)` in `main()` MUST flush the connect-back Tokio runtime in both scalar and set execution paths, preventing the 10 s join delay and the resulting Part:40 SIGABRT
