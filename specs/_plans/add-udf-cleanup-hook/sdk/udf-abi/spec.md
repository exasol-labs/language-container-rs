# Feature: udf-abi

Defines the `#[repr(C)]` ABI vtable, SDK fingerprint, vtable stability rules, and the `emit-arrow` feature boundary for the author-facing SDK. The spec-generation single-call hook vtable slots are specified separately in `sdk/udf-spec-abi`.

<!-- DELTA:CHANGED -->
## Background

The SDK ABI layer is the binary contract between a compiled UDF `.so` and the host runtime. The `#[repr(C)] ExaUdfVTable` carries an `abi_version`, an `sdk_fingerprint` (baked at build time from `SDK_VERSION:RUSTC_HASH`), a marker recording whether the UDF returns a value (RETURNS) or emits (EMITS), and function pointer slots for `run`, the optional `cleanup` hook, and optional single-call hooks. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable. The host loader checks `abi_version` and `sdk_fingerprint` at load time; a mismatch is a clean `AbiMismatch` error rather than silent UB.

The `UdfContext` trait-object vtable is ordered by method declaration. Every `UdfContext` method must be declared unconditionally (no `#[cfg(feature = ...)]`) so the vtable layout is identical in all build configurations — a feature-mismatched `.so` must fail the version check, not misdispatch calls. A change to the signature of an existing method changes that slot's calling convention while leaving the layout intact, and requires the same version increment as an added or reordered slot. The `emit-arrow` feature gates only the optional `arrow` dependency and the `EmitBatch` extension trait; it never gates `UdfContext` method declarations.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: cleanup annotation wires the cleanup slot

* *GIVEN* a function annotated `#[exasol_udf(cleanup(my_cleanup))]` where `my_cleanup` has the signature `fn(&mut dyn UdfContext) -> Result<(), UdfError>`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST generate an extern-C cleanup shim and wire it into the `cleanup` vtable slot as `Some`
* *AND* the shim MUST accept the `(ctx_ptr, error_out)` ABI of the `run` slot, reconstruct `&mut dyn UdfContext` from `ctx_ptr` via double-indirection, and call `my_cleanup(ctx)`
* *AND* on `Ok(())` the shim MUST return `0`; on `Err(e)` it MUST write the error's display text into a `malloc`-backed C string at `*error_out` when `error_out` is non-null and return `1`; on panic it MUST catch the unwind, leave `*error_out` untouched, and return `2`
* *AND* the hook MUST receive no argument besides the context, so it carries no information about whether the session succeeded, as the reference Python and Java `cleanup` functions carry none
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: cleanup absent leaves the slot None

* *GIVEN* a function annotated `#[exasol_udf]` with no `cleanup` section
* *WHEN* the crate is compiled as a cdylib
* *THEN* the `cleanup` vtable slot MUST be `None` and the macro MUST NOT generate a cleanup shim
* *AND* the runtime MUST answer the DB's `MT_CLEANUP` with `MT_FINISHED` directly, exactly as for a UDF built before the hook existed
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The cleanup slot replaces destroy in place, bumping the ABI version

* *GIVEN* an `ExaUdfVTable` whose `destroy: unsafe extern "C" fn()` slot takes no context and reports no error
* *WHEN* the vtable is compiled under this change
* *THEN* that slot MUST become `cleanup: Option<unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32>`, the signature of the `run` slot, at the same struct position, with every other field keeping its order
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped `10 → 11`, so a `.so` built against ABI 10 fails the loader's version check with `AbiMismatch` instead of having its no-argument `destroy` function called as a cleanup hook
<!-- /DELTA:NEW -->
