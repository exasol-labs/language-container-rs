# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the `#[repr(C)]` ABI vtable — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. The SDK fingerprint, baked at build time from the SDK version and compiler hash, is embedded in the vtable for load-time compatibility checking by the host. ABI version 3 changes the `virtual_schema_adapter_call` vtable slot to a 3-argument signature that includes the host `UdfContext` pointer, enabling VS adapters to call `ctx.connection(...)` and `ctx.connect_back(...)` from inside single-call mode. This is a hard binary incompatibility with ABI v2 — the loader rejects v2 artifacts.

`UdfContext` also exposes plain handshake metadata to UDF code. Beyond the typed column accessors it provides `memory_limit()`, the per-UDF-instance resident-memory limit in bytes sourced from `UdfMeta::maximal_memory_limit`; this is a defaulted accessor (not feature-gated) so existing implementations keep compiling, overridden by the host context bridge to return the live value.

## Scenarios

<!-- DELTA:NEW -->
### UdfContext exposes the per-instance memory limit

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF queries the resident-memory limit the database allotted to its VM instance
* *THEN* the trait MUST provide a `memory_limit(&self) -> u64` accessor returning the limit in bytes, sourced from `UdfMeta::maximal_memory_limit`
* *AND* the accessor MUST be a provided (defaulted) trait method returning `0` (denoting "no limit reported") so existing `UdfContext` implementations continue to compile without supplying it, mirroring how the SDK keeps the data-access surface backward compatible
* *AND* the accessor MUST NOT be gated behind the `connect-back` feature, because the limit is plain handshake metadata rather than a connect-back capability
* *AND* the host context bridge MUST override the default to return the exact byte value carried on `UdfMeta::maximal_memory_limit`
<!-- /DELTA:NEW -->
