# Feature: dispatch-run-loop

Orchestrates driving the scalar/set run loop over the wire protocol — covering iteration-shape dispatch, bridge row materialisation, context-contract enforcement, UDF error propagation, and connect-back availability. The `EmitBuffer`/`InputRowSet` rowset codec this loop drives (output packing, flush-threshold accounting, and any promoted fast-path formatter/parser) is specified separately in `runtime/rowset-codec`; the opt-in Arrow batch-emit path is specified separately in `runtime/emit-arrow-batch`. Loader validation and artifact resolution are specified separately in `runtime/dispatch-loader`. Single-call dispatch is specified separately in `runtime/dispatch-single-call`. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

The runtime drives dispatch via the pure protocol state machine after a `.so` has been loaded. The `HostContextBridge` adapts the host-internal `UdfMeta` and rowset codec into the `&dyn UdfContext` the UDF sees, threading handshake metadata (memory limit and the `exascript_info` identity/origin fields) in at construction so the bridge can override the SDK's defaulted accessors with live values.

The dispatcher MUST branch on the two `UdfMeta` iteration axes. The input axis (`input_iter`: `ExactlyOnce` = scalar, `Multiple` = set) selects who drives the input loop: for scalar the framework owns the per-row loop and invokes `run()` once per input row; for set the UDF drives its own loop via `ctx.next()` and `run()` is invoked once per input group. The output axis (`output_iter`: `ExactlyOnce` = RETURNS, `Multiple` = EMITS) selects the emit contract. The contracts match the reference Exasol containers' rejection semantics; shape is a runtime property (from the handshake metadata), not a Rust compile-time property, so enforcement is at runtime and surfaced through the `F-UDF-CL-RUST-` error-close path.

RETURNS output uses a value-return channel: the UDF function returns its value (`Some(v)` → one row, `None` → SQL NULL), the framework records it through `UdfContext::set_return` and emits the single row, and author-called `ctx.emit()` is banned in RETURNS context. EMITS output is unchanged — the UDF produces rows via `ctx.emit()`. The compiled output shape (from the `.so` vtable marker) is validated against `meta.output_iter` so a mismatch is a clear error rather than UB. The SDK exposes no `reset()` method, so the reference's "`reset()` banned in scalar" rule has no SDK surface to gate.

## Scenarios

### Scenario: Bridge materializes input rows into typed accessors

* *GIVEN* a `HostContextBridge` over a fake transport delivering one input batch of mixed column types, where the protobuf `ExascriptTableData` lays out values row-major within each type block (non-null cells only)
* *WHEN* a UDF calls `next` then the typed accessors
* *THEN* `next` MUST return `true` while rows remain and `false` when input is exhausted
* *AND* each typed accessor MUST return the correct value for the current row by advancing per-type cursors only on non-null cells — a NULL cell MUST NOT consume a slot in its type block
* *AND* a NULL cell MUST be returned as `Value::Null`

### Scenario: Scalar dispatch invokes the UDF once per input row

* *GIVEN* a loaded scalar UDF (`input_iter = ExactlyOnce`) whose input group arrives as one or more `MT_NEXT` batches
* *WHEN* the runtime drives the run phase
* *THEN* the runtime MUST invoke the UDF's `run` exactly once per input row, presenting that single row as the sole current row of the context
* *AND* the runtime MUST advance across every row of every `MT_NEXT` batch of the group until the DB answers `MT_NEXT` with `MT_DONE`, fetching each subsequent batch as the current batch drains
* *AND* the total number of `run` invocations MUST equal the group's total input row count, so no input row is skipped (correcting the defect where only the first row of each batch was processed)

### Scenario: Set dispatch invokes the UDF once per group spanning all input batches

* *GIVEN* a loaded set UDF (`input_iter = Multiple`) whose current input group spans more than one `MT_NEXT` batch, with a wire flusher closure that serializes the `EmitBuffer` to an `ExascriptTableData`, sends `MT_EMIT`, awaits the emit ack, and clears the buffer
* *WHEN* the UDF calls `ctx.next()` to iterate its input and emits output rows
* *THEN* the runtime MUST invoke `run` exactly once for the whole group
* *AND* `ctx.next()` MUST yield every row of the group across all its `MT_NEXT` batches, transparently fetching the next batch when the current batch drains
* *AND* `ctx.next()` MUST return `false` only at the group boundary (the DB answers `MT_NEXT` with `MT_DONE`), never at an intermediate batch boundary (correcting the defect where a group split across batches produced one partial result per batch)
* *AND* the bridge MUST accumulate emitted rows in the `EmitBuffer` and MUST trigger a mid-run `MT_EMIT` flush as soon as the buffer's running byte estimate reaches the `EMIT_BUFFER_LIMIT_BYTES` threshold of `4_000_000` bytes, rather than sending one frame per row or buffering an unbounded batch

### Scenario: Scalar input context rejects next()

* *GIVEN* a loaded UDF with `input_iter = ExactlyOnce` whose `run` calls `ctx.next()`
* *WHEN* the runtime drives the per-row loop and the UDF invokes `ctx.next()`
* *THEN* `ctx.next()` MUST return `Err(UdfError)` rather than advancing input, mirroring the reference containers' rejection of `next()`/`reset()` in scalar context
* *AND* the runtime MUST surface the error through the error-close path with the `F-UDF-CL-RUST-` prefix
* *AND* the SDK MUST NOT expose a `reset()` method, so no `reset()` gate is required

### Scenario: RETURNS output emits the value the UDF returned and bans emit()

* *GIVEN* a loaded UDF with `output_iter = ExactlyOnce` (RETURNS) whose function returns `Result<Option<Value>, UdfError>` and delivers the returned value through `UdfContext::set_return`
* *WHEN* a single `run` invocation returns `Some(v)`, returns `None`, or calls `ctx.emit()`
* *THEN* on `Some(v)` the runtime MUST emit exactly one output row carrying `v`
* *AND* on `None` the runtime MUST emit exactly one output row carrying SQL NULL
* *AND* any author call to `ctx.emit()` in RETURNS context MUST return `Err(UdfError)`, surfaced through the `F-UDF-CL-RUST-` error-close path (a genuine ban, not a count check)
* *AND* for `output_iter = Multiple` (EMITS) `ctx.emit()` MUST remain the output path accepting any number of rows, and `set_return` MUST NOT be used

### Scenario: Compiled output shape is validated against the DB output iteration type

* *GIVEN* a loaded `.so` whose `ExaUdfVTable` output-shape marker records whether the UDF returns a value (RETURNS) or emits (EMITS)
* *WHEN* the runtime resolves `meta.output_iter` at load or run
* *THEN* the runtime MUST accept the pairing when the marker matches (`return` value ↔ `ExactlyOnce`, `emit` ↔ `Multiple`)
* *AND* it MUST reject a mismatch — a value-returning UDF registered `EMITS`, or an emitting UDF registered `RETURNS` — with a clear error surfaced through the `F-UDF-CL-RUST-` error-close path, never silent misdispatch or UB

### Scenario: Emit buffer spans an input group across per-row and per-batch iteration

* *GIVEN* a scalar UDF that emits one row per input row over a group of at least `100_000` rows delivered in more than one `MT_NEXT` batch
* *WHEN* the runtime drives the per-row loop
* *THEN* emitted rows MUST accumulate in a single `EmitBuffer` that spans the whole group, flushing mid-group only when the running byte estimate reaches `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`), so output is batched rather than one `MT_EMIT` per row
* *AND* the runtime MUST send one tail `MT_EMIT` for any residual buffered rows before the group's `MT_DONE`, so no emitted row is lost
* *AND* the runtime MUST flush all of a group's output before that group's `MT_DONE`, so the DB attributes emitted rows to the correct group

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
