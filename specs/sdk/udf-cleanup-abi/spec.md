# Feature: udf-cleanup-abi

Defines the `#[exasol_udf(cleanup(...))]` annotation and the `cleanup` vtable slot it wires: shim generation, the slot's presence rules, and its replacement of the prior no-argument `destroy` slot. The general ABI vtable, fingerprint, and stability rules are specified separately in `sdk/udf-abi`.

## Background

A function annotated `#[exasol_udf(cleanup(my_cleanup))]`, where `my_cleanup` has the signature `fn(&mut dyn UdfContext) -> Result<(), UdfError>`, wires an extern-C shim into the `ExaUdfVTable.cleanup` slot: `Option<unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32>`, the signature of the `run` slot, at the struct position the prior no-argument `destroy: unsafe extern "C" fn()` slot held. The hook receives no argument besides the context, so it carries no information about whether the session succeeded, as the reference Python and Java `cleanup` functions carry none.

## Scenarios

### Scenario: cleanup annotation wires the cleanup slot

* *GIVEN* a function annotated `#[exasol_udf(cleanup(my_cleanup))]` where `my_cleanup` has the signature `fn(&mut dyn UdfContext) -> Result<(), UdfError>`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST generate an extern-C cleanup shim and wire it into the `cleanup` vtable slot as `Some`
* *AND* the shim MUST accept the `(ctx_ptr, error_out)` ABI of the `run` slot, reconstruct `&mut dyn UdfContext` from `ctx_ptr` via double-indirection, and call `my_cleanup(ctx)`
* *AND* on `Ok(())` the shim MUST return `0`; on `Err(e)` it MUST write the error's display text into a `malloc`-backed C string at `*error_out` when `error_out` is non-null and return `1`; on panic it MUST catch the unwind, leave `*error_out` untouched, and return `2`
* *AND* the hook MUST receive no argument besides the context, so it carries no information about whether the session succeeded, as the reference Python and Java `cleanup` functions carry none

### Scenario: cleanup absent leaves the slot None

* *GIVEN* a function annotated `#[exasol_udf]` with no `cleanup` section
* *WHEN* the crate is compiled as a cdylib
* *THEN* the `cleanup` vtable slot MUST be `None` and the macro MUST NOT generate a cleanup shim
* *AND* the runtime MUST answer the DB's `MT_CLEANUP` with `MT_FINISHED` directly, exactly as for a UDF built before the hook existed

### Scenario: The cleanup slot replaces destroy in place, bumping the ABI version

* *GIVEN* an `ExaUdfVTable` whose `destroy: unsafe extern "C" fn()` slot takes no context and reports no error
* *WHEN* the vtable is compiled under this change
* *THEN* that slot MUST become `cleanup: Option<unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32>`, the signature of the `run` slot, at the same struct position, with every other field keeping its order
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped `10 → 11`, so a `.so` built against ABI 10 fails the loader's version check with `AbiMismatch` instead of having its no-argument `destroy` function called as a cleanup hook
