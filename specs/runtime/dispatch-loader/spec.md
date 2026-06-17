# Feature: dispatch-loader

Validates and loads a precompiled UDF `.so` artifact before dispatch — covering ABI version and SDK fingerprint gating, artifact path resolution from script options, and the unsupported JIT compilation path.

## Background

The runtime loads a precompiled `.so` (Option A), gating on ABI version and SDK fingerprint checks before calling into any UDF code. JIT compilation (Option C) remains unsupported in v2 and surfaces an `UnsupportedFeature` error through the protocol close path. Artifact resolution reads the `%udf_object` option from the script source to locate the `.so` on BucketFS.

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
