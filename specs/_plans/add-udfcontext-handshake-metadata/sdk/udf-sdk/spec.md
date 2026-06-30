# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the row-based emit surface — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`; the ABI vtable and `emit-arrow` feature boundary are specified in `sdk/udf-abi`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`.

`UdfContext` exposes plain handshake metadata to UDF code. Beyond the typed column accessors it provides `memory_limit()` and the `exascript_info` identity/origin accessors (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`), each sourced from `UdfMeta`; these are defaulted accessors (not feature-gated) so existing implementations keep compiling, overridden by the host context bridge to return the live value.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: UdfContext exposes handshake identity and origin metadata

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF queries the handshake metadata the database delivered in `exascript_info`
* *THEN* the trait MUST provide the numeric accessors `session_id(&self) -> u64`, `statement_id(&self) -> u32`, `node_id(&self) -> u32`, `node_count(&self) -> u32`, and `vm_id(&self) -> u64`, the owned-string accessors `database_name`, `database_version`, `script_name`, and `script_schema` (returning owned values, not borrows, because they cross the `.so` vtable boundary), and the optional accessors `current_user`, `current_schema`, and `scope_user` returning `Option<String>` to mirror the `optional` proto fields, each sourced from the corresponding `UdfMeta` field
* *AND* every accessor MUST be a provided (defaulted) trait method, mirroring `memory_limit()`, so existing `UdfContext` implementations continue to compile without supplying it
* *AND* the numeric accessors MUST default to `0`, the owned-string accessors MUST default to the empty string, and the optional accessors MUST default to `None`, each denoting "not reported"
* *AND* none of the accessors MAY be gated behind the `connect-back` feature, because handshake metadata is plain DB-supplied context rather than a connect-back capability
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: UdfContext exposes the per-instance memory limit

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF queries the resident-memory limit the database allotted to its VM instance
* *THEN* the trait MUST provide a `memory_limit(&self) -> u64` accessor returning the limit in bytes, sourced from `UdfMeta::maximal_memory_limit`
* *AND* the accessor MUST be a provided (defaulted) trait method returning `0` (denoting "no limit reported") so existing `UdfContext` implementations continue to compile without supplying it, mirroring how the SDK keeps the data-access surface backward compatible
* *AND* the accessor MUST NOT be gated behind the `connect-back` feature, because the limit is plain handshake metadata rather than a connect-back capability
* *AND* the accessor MUST follow the same defaulted-accessor pattern as the identity and origin metadata accessors, with the host context bridge overriding the default to return the exact byte value carried on `UdfMeta::maximal_memory_limit`
<!-- /DELTA:CHANGED -->
