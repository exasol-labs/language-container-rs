# Feature: udf-sdk

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. The host owns the `UdfContext` implementation and reads any state the UDF leaves behind through it.

## Background

A UDF function returning `Err(UdfError)` from `run` is mapped by the generated run shim to a non-zero error code. Before this change the shim discarded the `UdfError` value entirely, so the host could only report a generic error code. The fix carries the error text out of the shim through a dedicated out-pointer parameter on the vtable `run` slot rather than through any `UdfContext` trait method: the `run` signature gains an `error_out: *mut *mut c_char` parameter the shim fills with a heap-allocated C string on `Err`. This changes the vtable layout, so the ABI version is bumped and the `UdfContext` trait is left entirely unchanged.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Run shim surfaces UDF error text via an out-pointer parameter

* *GIVEN* the `ExaUdfVTable.run` slot and the `#[exasol_udf]` generated run shim
* *WHEN* a UDF function returns `Err(UdfError)` from `run`
* *THEN* the `ExaUdfVTable.run` function pointer signature MUST take a second parameter `error_out: *mut *mut c_char` in addition to the existing `ctx: *mut c_void`
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped because the vtable `run` layout changed, so the host rejects `.so` files built against the previous ABI
* *AND* the generated run shim MUST, on the `Err(e)` arm and when `error_out` is non-null, write a heap-allocated, caller-freed C string holding the error's display text to `*error_out` before returning the non-zero error code
* *AND* the `UdfContext` trait MUST NOT gain any new method for this purpose, so existing host bridge `UdfContext` implementations and their connect-back `last_error` plumbing are unchanged
<!-- /DELTA:NEW -->
