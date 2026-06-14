# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the `#[repr(C)]` ABI vtable — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. The SDK fingerprint, baked at build time from the SDK version and compiler hash, is embedded in the vtable for load-time compatibility checking by the host. ABI version 3 changes the `virtual_schema_adapter_call` vtable slot to a 3-argument signature that includes the host `UdfContext` pointer, enabling VS adapters to call `ctx.connection(...)` and `ctx.connect_back(...)` from inside single-call mode. This is a hard binary incompatibility with ABI v2 — the loader rejects v2 artifacts.

## Scenarios

### Scenario: Value and ExaType cover the v1 column types

* *GIVEN* the SDK `value` module
* *WHEN* a UDF reads or emits a column
* *THEN* `Value` MUST provide strongly typed variants for `Null`, `Int32(i32)`, `Int64(i64)`, `Double(f64)`, `Numeric(Decimal)`, `Bool(bool)`, `String(String)`, `Date(NaiveDate)`, and `Timestamp(NaiveDateTime)`, where `Numeric` carries a `Decimal { unscaled: i128, scale: u8 }` newtype and `Date`/`Timestamp` carry `chrono::NaiveDate`/`NaiveDateTime` (NOT `String`)
* *AND* the single canonical `ExaType` MUST live in the SDK `value` module and provide matching descriptors including `Numeric { precision, scale }` and `String { size }`
* *AND* `exa-zmq-protocol` MUST re-use the SDK `ExaType` rather than defining its own duplicate enum

### Scenario: Decimal is constructible from string and float without precision loss

* *GIVEN* the SDK `Decimal` newtype
* *WHEN* a UDF or the runtime constructs a decimal from the proto wire form
* *THEN* `Decimal::try_from(&str)` MUST parse a signed decimal literal such as `"-1.000000000000000001"` into `unscaled` and `scale` with no precision loss for up to 38 significant digits
* *AND* `Decimal::try_from(f64)` MUST be provided for callers holding a floating-point value, returning `UdfError::Type` (or a dedicated decimal error) for non-finite inputs
* *AND* `Decimal::to_string` MUST round-trip back to the canonical decimal wire form so emit serialization is lossless
* *AND* a value whose `scale` is `0` MUST render with no decimal point

### Scenario: UdfContext exposes typed accessors and row iteration

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF inspects and reads its input
* *THEN* the trait MUST provide `next`, `reset`, `emit`, and column introspection (`column_count`, `column_name`, `column_type`, `column_index`)
* *AND* it MUST provide typed accessors `get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, `get_timestamp`, and `get_value`, each returning `Result<Option<T>, UdfError>` where a SQL NULL maps to `Ok(None)` and a matching cell maps to `Ok(Some(value))`
* *AND* `get_i64` MUST additionally accept an integral `Value::Numeric` cell (because Exasol delivers `BIGINT` as `PB_NUMERIC`), returning `UdfError::Type` only when the decimal has a non-zero fractional part
* *AND* a typed accessor invoked on a column whose `Value` variant does not match the requested type (and is not the documented `Numeric`→`i64` case) MUST return `UdfError::Type` rather than silently coercing

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
