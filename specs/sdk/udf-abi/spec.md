# Feature: udf-abi

Defines the `#[repr(C)]` ABI vtable, SDK fingerprint, vtable stability rules, and the `emit-arrow` feature boundary for the author-facing SDK. The spec-generation single-call hook vtable slots are specified separately in `sdk/udf-spec-abi`. The `cleanup` annotation and vtable slot are specified separately in `sdk/udf-cleanup-abi`.

## Background

The SDK ABI layer is the binary contract between a compiled UDF `.so` and the host runtime. The `#[repr(C)] ExaUdfVTable` carries an `abi_version`, an `sdk_fingerprint` (baked at build time from `SDK_VERSION:RUSTC_HASH`), a marker recording whether the UDF returns a value (RETURNS) or emits (EMITS), and function pointer slots for `run`, the optional `cleanup` hook, and optional single-call hooks. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable. The host loader checks `abi_version` and `sdk_fingerprint` at load time; a mismatch is a clean `AbiMismatch` error rather than silent UB.

The `UdfContext` trait-object vtable is ordered by method declaration. Every `UdfContext` method must be declared unconditionally (no `#[cfg(feature = ...)]`) so the vtable layout is identical in all build configurations — a feature-mismatched `.so` must fail the version check, not misdispatch calls. A change to the signature of an existing method changes that slot's calling convention while leaving the layout intact, and requires the same version increment as an added or reordered slot. The `emit-arrow` feature gates only the optional `arrow` dependency and the `EmitBatch` extension trait; it never gates `UdfContext` method declarations.

## Scenarios

### Scenario: ABI version is bumped so a stale .so is rejected, not misdispatched

* *GIVEN* the loader compares a loaded `.so`'s `ExaUdfVTable.abi_version` against the host's `EXA_UDF_ABI_VERSION`
* *WHEN* the `UdfContext` trait layout changes under this delta (feature-independent vtable)
* *THEN* `EXA_UDF_ABI_VERSION` MUST be incremented (4 → 5), so a `.so` compiled against the previous, feature-dependent layout fails the loader's existing version check with a clear `AbiMismatch` error instead of silently calling the wrong vtable slot
* *AND* the `#[repr(C)] ExaUdfVTable` struct field order MUST remain unchanged, so the version check itself remains readable across the boundary (the bump signals the `dyn UdfContext` layout change, which the struct does not otherwise encode)

### Scenario: SDK fingerprint is baked at build time

* *GIVEN* the SDK `build.rs`
* *WHEN* the crate is compiled
* *THEN* it MUST set an `EXA_SDK_FINGERPRINT` value of the form `"SDK_VERSION:RUSTC_HASH\0"`
* *AND* the macro-generated vtable MUST embed that exact fingerprint string in its `sdk_fingerprint` field

### Scenario: vs_adapter annotation wires the virtual_schema_adapter_call slot

* *GIVEN* a struct annotated `#[exasol_udf(vs_adapter(my_adapter_fn))]` where `my_adapter_fn` has the signature `fn(&mut dyn UdfContext, &str) -> Result<String, UdfError>`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST generate an `__exa_vs_adapter_shim` extern-C function and wire it into the `virtual_schema_adapter_call` vtable slot
* *AND* the shim MUST accept the 3-argument ABI `(ctx_ptr, json_arg, result)`, reconstruct `&mut dyn UdfContext` from `ctx_ptr` via double-indirection, and call `my_adapter_fn(ctx, json)`
* *AND* on `Ok(s)` the shim MUST write `s` into a `malloc`-backed C string at `*result` and return `0`; on `Err(e)` write the error and return `1`; on panic catch the unwind and return `2`
* *AND* interior NUL bytes in the response string MUST be replaced with U+FFFD before writing to `*result`

### Scenario: vs_adapter absent leaves slot None for backward compatibility

* *GIVEN* a struct annotated `#[exasol_udf]` with no `vs_adapter` clause
* *WHEN* the crate is compiled as a cdylib
* *THEN* the `virtual_schema_adapter_call` vtable slot MUST be `None`
* *AND* the runtime MUST reply `MT_UNDEFINED_CALL` when the DB invokes `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`, preserving backward compatibility with UDFs compiled before this change

### Scenario: Run shim surfaces UDF error text via an out-pointer parameter

* *GIVEN* the `ExaUdfVTable.run` slot and the `#[exasol_udf]` generated run shim
* *WHEN* a UDF function returns `Err(UdfError)` from `run`
* *THEN* the `ExaUdfVTable.run` function pointer signature MUST take a second parameter `error_out: *mut *mut c_char` in addition to the existing `ctx: *mut c_void`
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped because the vtable `run` layout changed, so the host rejects `.so` files built against the previous ABI
* *AND* the generated run shim MUST, on the `Err(e)` arm and when `error_out` is non-null, write a heap-allocated, caller-freed C string holding the error's display text to `*error_out` before returning the non-zero error code; ownership of the allocation follows the `malloc`/`libc::free` C-allocator convention used by all other vtable result strings
* *AND* the `UdfContext` trait MUST NOT gain any new method for this purpose, so existing host bridge `UdfContext` implementations and their connect-back `last_error` plumbing are unchanged

### Scenario: UdfContext vtable layout is feature-independent

* *GIVEN* the `exasol-udf-sdk` crate built with any combination of cargo features
* *WHEN* the `UdfContext` trait is compiled
* *THEN* every method — including `cluster_ip`, `connection`, `connect_back`, and `emit_record_batch_ffi` — MUST be declared **unconditionally** (no `#[cfg(feature = ...)]` on any trait method), each with a default implementation returning `UdfError::Unimplemented`, so the method set and declaration order (hence the `dyn UdfContext` vtable layout) are identical in every build configuration
* *AND* `connection` and `connect_back` MUST reference `ConnectionObject` / `ExaConnection` from the unconditionally compiled `connection` and `connect_back` modules (see `sdk/connection`, `sdk/connect-back`), so their signatures compile without any feature gate
* *AND* `emit_record_batch_ffi(&mut self, array: *mut c_void, schema: *mut c_void) -> Result<(), UdfError>` MUST name no `arrow` type, so it can be declared unconditionally even when the `arrow` dependency is absent
* *AND* a UDF `.so` and the host built with different feature sets MUST resolve every `UdfContext` method to the same vtable slot, so `ctx.emit_batch()` from an `emit-arrow`-only UDF dispatches to the host's `emit_record_batch_ffi` (not `cluster_ip`) and emits all rows (issue #31)

### Scenario: emit-arrow gates only the arrow dependency and the RecordBatch ext-trait

* *GIVEN* the SDK compiled without the `emit-arrow` feature
* *WHEN* the crate is built
* *THEN* the optional `arrow` dependency MUST NOT be compiled and the `EmitBatch` extension trait (`emit_batch(&RecordBatch)`) MUST NOT be present, because it is the only API that names an `arrow` type
* *AND* the `UdfContext::emit_record_batch_ffi` trait method MUST still be present, so the vtable is unchanged whether or not `emit-arrow` is enabled
* *AND* the row-based `emit(&mut self, values: Vec<Value>)` MUST remain a required trait method, so a UDF MAY mix `emit` and `emit_batch` within one `run`

### Scenario: Return channel adds set_return and an output-shape marker, bumping the ABI version

* *GIVEN* the value-return channel that delivers a RETURNS UDF's returned value to the host
* *WHEN* the `UdfContext` trait and `ExaUdfVTable` are compiled under this change
* *THEN* `UdfContext` MUST gain a `set_return(&mut self, value: Option<Value>) -> Result<(), UdfError>` method declared unconditionally with a default returning `UdfError::Unimplemented`, so the `dyn UdfContext` vtable layout stays feature-independent
* *AND* `ExaUdfVTable` MUST carry an output-shape marker (RETURNS versus EMITS) that the loader/runtime validates against `meta.output_iter`
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped `6 → 7` because both the `dyn UdfContext` layout and the `ExaUdfVTable` fields changed, so a `.so` built against ABI 6 fails the loader's version check with a clear `AbiMismatch` error instead of misdispatching
* *AND* the `run` vtable function-pointer signature MUST remain `(ctx: *mut c_void, error_out: *mut *mut c_char)` — the returned value crosses through the existing trait-object `set_return` slot, not a new `run` parameter

### Scenario: emit_batch crosses the .so boundary through the Arrow C Data Interface

* *GIVEN* the SDK compiled with the `emit-arrow` feature enabled
* *WHEN* a UDF author calls `ctx.emit_batch(&batch)` on a `&mut dyn UdfContext`
* *THEN* the blanket `EmitBatch` extension trait MUST export the batch in the caller (UDF) crate as an `FFI_ArrowArray` and an `FFI_ArrowSchema` and pass pointers to both to `emit_record_batch_ffi`
* *AND* no Rust `arrow` type MUST cross the `.so` boundary, because the UDF and the host link separate `arrow` copies whose `TypeId`s and trait-object vtables differ
* *AND* the host MUST take ownership of the array, so its release callback runs exactly once
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped `11 → 12`, so a `.so` built against ABI 11 fails the loader's version check with `AbiMismatch`
