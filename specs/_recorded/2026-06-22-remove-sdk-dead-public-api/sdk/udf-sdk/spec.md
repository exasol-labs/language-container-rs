# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the `#[repr(C)]` ABI vtable — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. The SDK fingerprint, baked at build time from the SDK version and compiler hash, is embedded in the vtable for load-time compatibility checking by the host. ABI version 3 changes the `virtual_schema_adapter_call` vtable slot to a 3-argument signature that includes the host `UdfContext` pointer, enabling VS adapters to call `ctx.connection(...)` and `ctx.connect_back(...)` from inside single-call mode. This is a hard binary incompatibility with ABI v2 — the loader rejects v2 artifacts.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Value and ExaType cover the v1 column types

* *GIVEN* the SDK `value` module
* *WHEN* a UDF reads or emits a column
* *THEN* it MUST provide strongly typed variants for `Null`, `Double`, `Int32`, `Int64`, `Numeric`, `Bool`, `String`, `Date`, and `Timestamp`, where `Numeric` carries a `Decimal` newtype and `Date`/`Timestamp` carry `NaiveDate`/`NaiveDateTime` (NOT `i64`)
* *AND* the single canonical `ExaType` MUST live in the SDK `value` module and provide matching descriptors including `precision` and `scale`
* *AND* MUST re-use the SDK `ExaType` rather than defining its own duplicate enum
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Decimal is constructible from string and float without precision loss

* *GIVEN* the SDK `Decimal` newtype
* *WHEN* a UDF or the runtime constructs a decimal from the proto wire form
* *THEN* `Decimal::try_from(f64)` MUST be provided for callers holding a floating-point value, returning `UdfError::Type` for non-finite inputs
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Decimal is constructible from string without precision loss

* *GIVEN* the SDK `Decimal` newtype
* *WHEN* a UDF or the runtime constructs a decimal from the proto wire form
* *THEN* `Decimal::try_from(&str)` MUST parse a signed decimal literal such as `"-1.000000000000000001"` into `unscaled` and `scale` with no precision loss for up to 38 significant digits
* *AND* `TryFrom<&str>` MUST be provided as the canonical construction path, returning a `UdfError::Type` for malformed input
* *AND* `Decimal::to_string` MUST round-trip back to the canonical decimal wire form so emit serialization is lossless
* *AND* a value whose `scale` is `0` MUST render with no decimal point
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: UdfContext exposes typed accessors and row iteration

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF inspects and reads its input
* *THEN* the trait MUST provide `num_columns`, `get`, `emit`, and `next` as required methods
* *AND* it MUST provide typed accessors `get_value`, `get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, and `get_timestamp`, each returning `Result<Option<T>, UdfError>` where a SQL NULL maps to `Ok(None)` and a matching cell maps to `Ok(Some(…))`
* *AND* `get_i64` MUST additionally accept an integral `Numeric` cell (because Exasol delivers `BIGINT` as `PB_NUMERIC`), returning `Err(UdfError::Type)` only when the decimal has a non-zero fractional part
* *AND* a typed accessor invoked on a column whose `Value` variant does not match the requested type (and is not the documented `Numeric`→`i64` case) MUST return `Err(UdfError::Type)` rather than silently coercing
<!-- /DELTA:CHANGED -->
