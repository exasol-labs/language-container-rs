# Feature: dispatch-loader

Validates and loads a precompiled UDF `.so` artifact before dispatch — covering named entry-point resolution, ABI version and SDK fingerprint gating, artifact path resolution from script options, and the unsupported JIT compilation path.

## Background

The runtime loads a precompiled `.so` (Option A), gating on ABI version and SDK fingerprint checks before calling into any UDF code. This delta changes entry resolution: the loader now resolves `__exa_udf_entry_<SCRIPT_NAME>` using the bare object name the database supplies in the handshake (`UdfMeta.script_name`), so one `.so` may carry many UDFs. A missing named symbol — including legacy `.so` files that export only the bare `__exa_udf_entry` — produces a clear rebuild-hint error surfaced through the protocol close path; there is no fallback. JIT compilation (Option C) remains unsupported.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Loader accepts a matching .so and calls create

* *GIVEN* a UDF `.so` built against the host's SDK fingerprint and `abi_version = 1`, exporting `__exa_udf_entry_DOUBLE_IT`
* *AND* the database sent `script_name = "DOUBLE_IT"` in the handshake metadata
* *WHEN* the loader opens it and resolves the named entry symbol `__exa_udf_entry_DOUBLE_IT` derived from the script name
* *THEN* it MUST verify `abi_version` equals `EXA_UDF_ABI_VERSION`
* *AND* it MUST verify the vtable `sdk_fingerprint` matches the host fingerprint
* *AND* it MUST call `create` and return a handle holding the `Library` alive
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Loader returns clear error when named entry point is absent

* *GIVEN* a UDF `.so` that exports `__exa_udf_entry_DOUBLE_IT` but not `__exa_udf_entry_MISSING`
* *AND* the database sent `script_name = "MISSING"` in the handshake metadata
* *WHEN* the loader resolves the named entry symbol `__exa_udf_entry_MISSING`
* *THEN* it MUST return a clear error of the form `no entry point found for script 'MISSING'; hint: rebuild with sdk >= 0.14.0`
* *AND* it MUST NOT call `create` or dereference any function pointers
* *AND* the error MUST be surfaced through the protocol close path with the `F-UDF-CL-RUST-` prefix
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Legacy single-entry .so fails to load

* *GIVEN* a UDF `.so` built with an SDK older than 0.14.0 that exports only the bare `__exa_udf_entry` symbol
* *AND* the database sent any `script_name`
* *WHEN* the loader resolves the named entry symbol `__exa_udf_entry_<SCRIPT_NAME>`
* *THEN* the named symbol MUST be absent and the loader MUST return the `no entry point found for script '<NAME>'; hint: rebuild with sdk >= 0.14.0` error
* *AND* the loader MUST NOT fall back to the bare `__exa_udf_entry` symbol
<!-- /DELTA:NEW -->

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
