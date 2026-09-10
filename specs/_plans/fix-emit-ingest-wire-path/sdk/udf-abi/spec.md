# Feature: udf-abi

Defines the `#[repr(C)]` ABI vtable, SDK fingerprint, vtable stability rules, and the `emit-arrow` feature boundary for the author-facing SDK.

## Background

The SDK ABI layer is the binary contract between a compiled UDF `.so` and the host runtime. The `#[repr(C)] ExaUdfVTable` carries an `abi_version`, an `sdk_fingerprint` (baked at build time from `SDK_VERSION:RUSTC_HASH`), a marker recording whether the UDF returns a value (RETURNS) or emits (EMITS), and function pointer slots for `run`, `destroy`, and optional single-call hooks. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable. The host loader checks `abi_version` and `sdk_fingerprint` at load time; a mismatch is a clean `AbiMismatch` error rather than silent UB.

The `UdfContext` trait-object vtable is ordered by method declaration. Every `UdfContext` method must be declared unconditionally (no `#[cfg(feature = ...)]`) so the vtable layout is identical in all build configurations — a feature-mismatched `.so` must fail the version check, not misdispatch calls. The `emit-arrow` feature gates only the optional `arrow` dependency and the `EmitBatch` extension trait; it never gates `UdfContext` method declarations.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Owned-row emit widens the UdfContext vtable and bumps the ABI version

* *GIVEN* the `UdfContext` trait-object vtable, whose slot order follows method declaration order
* *WHEN* `emit_owned` is added to the trait
* *THEN* `EXA_UDF_ABI_VERSION` MUST be incremented (7 → 8), so a `.so` built against the previous layout fails the loader's version check with a clear `AbiMismatch` error instead of dispatching through a shifted slot
* *AND* `emit_owned` MUST be declared unconditionally, with no `#[cfg(feature = ...)]` gate, preserving the feature-independent vtable layout
* *AND* the `#[repr(C)] ExaUdfVTable` field order MUST remain unchanged, because the bump alone signals the `dyn UdfContext` layout change
<!-- /DELTA:NEW -->
</content>
