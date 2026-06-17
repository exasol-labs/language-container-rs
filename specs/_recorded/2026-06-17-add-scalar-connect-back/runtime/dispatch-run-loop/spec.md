# Feature: dispatch-run-loop

Orchestrates loading a UDF `.so` and driving the scalar/set run loop over the wire protocol — covering loader validation, artifact resolution, bridge row materialisation, emit buffering, and UDF error propagation. Single-call dispatch is specified separately in `runtime/dispatch-single-call`. The connect-back host implementation is specified separately in `runtime/connect-back`.

## Background

The runtime loads a precompiled `.so` (Option A), gating on ABI version and SDK fingerprint, then drives dispatch via the pure protocol state machine. JIT compilation remains unsupported in v2 (`compiler.rs` returns `UnsupportedFeature`). The rowset codec (`InputRowSet`/`EmitBuffer`) switches from column-major packing with NULL placeholders to row-major packing where NULL cells occupy no slot in their type block, and output values are packed by declared column `ExaType` rather than by runtime `Value` variant.

## Scenarios

### Scenario: Loader accepts a matching .so and calls create

* *GIVEN* a UDF `.so` built against the host's SDK fingerprint and `abi_version = 1`
* *WHEN* the loader opens it and resolves `__exa_udf_entry`
* *THEN* it MUST verify `abi_version` equals `EXA_UDF_ABI_VERSION`
* *AND* it MUST verify the vtable `sdk_fingerprint` matches the host fingerprint
* *AND* it MUST call `create` and return a handle holding the `Library` alive

### Scenario: Loader rejects an ABI version mismatch

* *GIVEN* a UDF `.so` whose vtable reports an `abi_version` other than `1`
* *WHEN* the loader validates the vtable
* *THEN* it MUST return a clear error identifying the version mismatch
* *AND* it MUST NOT call `create` or dereference any function pointers

### Scenario: Loader rejects a fingerprint mismatch

* *GIVEN* a UDF `.so` whose vtable `sdk_fingerprint` does not match the host fingerprint
* *WHEN* the loader validates the vtable
* *THEN* it MUST return a clear error identifying the fingerprint mismatch rather than producing undefined behavior
* *AND* it MUST NOT call `create`

### Scenario: Artifact path is parsed from the udf_object option

* *GIVEN* a script source containing `%udf_object /buckets/bfsdefault/default/udfs/libudf.so`
* *WHEN* the runtime resolves the artifact
* *THEN* it MUST extract the `.so` path from the `%udf_object` option
* *AND* it MUST load that path via the loader without invoking the JIT compiler

### Scenario: JIT compilation is unsupported in v1

* *GIVEN* a script source with no `%udf_object` option (JIT path)
* *WHEN* the runtime attempts to resolve the artifact
* *THEN* the compiler entry point MUST return an unsupported-feature error
* *AND* the error MUST be surfaced through the protocol close path with the `F-UDF-CL-RUST-` prefix

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

* *GIVEN* a loaded set UDF and a `HostContextBridge` with `iter_type = Multiple`
* *WHEN* the UDF iterates all input rows and emits a filtered subset
* *THEN* `emit` MUST accumulate output and flush `MT_EMIT` in batches rather than one frame per row
* *AND* the total emitted row count MUST equal the number of `emit` calls the UDF made

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

<!-- DELTA:NEW -->
### Scenario: Connect-back is available identically in scalar and set dispatch

* *GIVEN* a runtime driving either a scalar UDF (`iter_type = ExactlyOnce`) or a set UDF (`iter_type = Multiple`) with the connect-back feature enabled
* *WHEN* the UDF calls `ctx.connection()` or `ctx.connect_back()` during its `run` method
* *THEN* the runtime MUST handle the connect-back MT_IMPORT exchange and session open identically for both `iter_type` values — there MUST be no scalar-specific restriction, guard, or branch that prevents connect-back in the scalar path
* *AND* the ZMQ socket MUST be idle during `run` in both cases (blocked awaiting UDF function return), making the MT_IMPORT exchange safe in both scalar and set dispatch
* *AND* `std::process::exit(0)` in `main()` MUST flush the connect-back Tokio runtime in both scalar and set execution paths, preventing the 10 s join delay and the resulting Part:40 SIGABRT
<!-- /DELTA:NEW -->
