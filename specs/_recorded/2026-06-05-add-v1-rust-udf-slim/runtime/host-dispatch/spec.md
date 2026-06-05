# Feature: host-dispatch

Implements the host runtime that loads a precompiled UDF `.so` from BucketFS, gates it on ABI and fingerprint compatibility, bridges the wire protocol to the SDK's `UdfContext`, and drives the scalar and set/EMITS dispatch loops with Arrow-backed input buffering and emit batching.

## Background

The runtime links `exa-zmq-protocol` and `exasol-udf-sdk`. It resolves the artifact from the `%udf_object <path>` script option (Option A, precompiled), `dlopen`s it via `libloading`, and validates the vtable before calling `create`. JIT compilation (Option C) is out of scope for v1 — the compiler entry point MUST return an unsupported error. Connect-back and single-call (`SC_FN_*`) dispatch are also out of scope for v1.

The `HostContextBridge` implements `UdfContext`: its `next` issues `MT_NEXT` and materializes input into Arrow column builders, its typed accessors read the current row, and its `emit` accumulates output and flushes `MT_EMIT` in batches. The `Library` is held alive for the session and dropped after `destroy`. UDF failures and panic error codes are serialized into the protocol close path with the `F-UDF-CL-RUST-####` prefix.

<!-- NEW -->

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

* *GIVEN* a `HostContextBridge` over a fake transport delivering one input batch of mixed column types
* *WHEN* a UDF calls `next` then the typed accessors
* *THEN* `next` MUST return `true` while rows remain and `false` when input is exhausted
* *AND* each typed accessor MUST return the correct value for the current row, mapping the eight PB column types to their Arrow arrays
* *AND* a NULL cell MUST be returned as `None`

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

<!-- /NEW -->
