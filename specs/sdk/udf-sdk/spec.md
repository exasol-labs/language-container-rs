# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the `#[repr(C)]` ABI vtable — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. The SDK fingerprint, baked at build time from the SDK version and compiler hash, is embedded in the vtable for load-time compatibility checking by the host. ABI version 3 changes the `virtual_schema_adapter_call` vtable slot to a 3-argument signature that includes the host `UdfContext` pointer, enabling VS adapters to call `ctx.connection(...)` and `ctx.connect_back(...)` from inside single-call mode. This is a hard binary incompatibility with ABI v2 — the loader rejects v2 artifacts.

`UdfContext` also exposes plain handshake metadata to UDF code. Beyond the typed column accessors it provides `memory_limit()`, the per-UDF-instance resident-memory limit in bytes sourced from `UdfMeta::maximal_memory_limit`; this is a defaulted accessor (not feature-gated) so existing implementations keep compiling, overridden by the host context bridge to return the live value.

## Scenarios

### Scenario: Value and ExaType cover the v1 column types

* *GIVEN* the SDK `value` module
* *WHEN* a UDF reads or emits a column
* *THEN* it MUST provide strongly typed variants for `Null`, `Double`, `Int32`, `Int64`, `Numeric`, `Bool`, `String`, `Date`, and `Timestamp`, where `Numeric` carries a `Decimal` newtype and `Date`/`Timestamp` carry `NaiveDate`/`NaiveDateTime` (NOT `i64`)
* *AND* the single canonical `ExaType` MUST live in the SDK `value` module and provide matching descriptors including `precision` and `scale`
* *AND* MUST re-use the SDK `ExaType` rather than defining its own duplicate enum

### Scenario: Decimal is constructible from string without precision loss

* *GIVEN* the SDK `Decimal` newtype
* *WHEN* a UDF or the runtime constructs a decimal from the proto wire form
* *THEN* `Decimal::try_from(&str)` MUST parse a signed decimal literal such as `"-1.000000000000000001"` into `unscaled` and `scale` with no precision loss for up to 38 significant digits
* *AND* `TryFrom<&str>` MUST be provided as the canonical construction path, returning a `UdfError::Type` for malformed input
* *AND* `Decimal::to_string` MUST round-trip back to the canonical decimal wire form so emit serialization is lossless
* *AND* a value whose `scale` is `0` MUST render with no decimal point

### Scenario: UdfContext exposes typed accessors and row iteration

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF inspects and reads its input
* *THEN* the trait MUST provide `num_columns`, `get`, `emit`, and `next` as required methods
* *AND* it MUST provide typed accessors `get_value`, `get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, and `get_timestamp`, each returning `Result<Option<T>, UdfError>` where a SQL NULL maps to `Ok(None)` and a matching cell maps to `Ok(Some(…))`
* *AND* `get_i64` MUST additionally accept an integral `Numeric` cell (because Exasol delivers `BIGINT` as `PB_NUMERIC`), returning `Err(UdfError::Type)` only when the decimal has a non-zero fractional part
* *AND* a typed accessor invoked on a column whose `Value` variant does not match the requested type (and is not the documented `Numeric`→`i64` case) MUST return `Err(UdfError::Type)` rather than silently coercing

### Scenario: UdfRun default single-call hooks return Unimplemented

* *GIVEN* a struct that implements `UdfRun` providing only `run`
* *WHEN* a single-call hook (`default_output_columns`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`, `virtual_schema_adapter_call`) is invoked
* *THEN* the default implementation MUST return `UdfError::Unimplemented`
* *AND* the trait MUST compile without the author providing those hooks

### Scenario: ABI constants and vtable layout are stable

* *GIVEN* the SDK `abi` module
* *WHEN* the vtable type is referenced by the host loader and the macro
* *THEN* `EXA_UDF_ABI_VERSION` MUST equal `3`
* *AND* `ExaUdfVTable` MUST be `#[repr(C)]` with fields `abi_version`, `sdk_fingerprint`, `run`, `destroy`, and optional `default_output_columns`, `virtual_schema_adapter_call`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`, `annotated_input_schema`, `annotated_output_schema`
* *AND* the `virtual_schema_adapter_call` slot MUST have the 3-argument signature `(ctx: *mut c_void, json_arg: *const c_char, result: *mut *mut c_char) -> i32`, where `ctx` is the same double-indirected `&mut dyn UdfContext` pointer the host passes to `run`

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

### Scenario: UdfContext exposes the per-instance memory limit

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF queries the resident-memory limit the database allotted to its VM instance
* *THEN* the trait MUST provide a `memory_limit(&self) -> u64` accessor returning the limit in bytes, sourced from `UdfMeta::maximal_memory_limit`
* *AND* the accessor MUST be a provided (defaulted) trait method returning `0` (denoting "no limit reported") so existing `UdfContext` implementations continue to compile without supplying it, mirroring how the SDK keeps the data-access surface backward compatible
* *AND* the accessor MUST NOT be gated behind the `connect-back` feature, because the limit is plain handshake metadata rather than a connect-back capability
* *AND* the host context bridge MUST override the default to return the exact byte value carried on `UdfMeta::maximal_memory_limit`
