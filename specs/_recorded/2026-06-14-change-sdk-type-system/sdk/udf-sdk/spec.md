# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext`/`UdfRun` traits and the `Value`/`ExaType`/`Decimal` type model — that UDF crates depend on without linking the host runtime or exarrow-rs. This delta replaces the string-based `Value` variants with strongly typed Rust types, adds the full set of typed `UdfContext` getters, and makes the SDK the single canonical home of `ExaType`.

## Background

The SDK is a pure contract crate. Today `Value::Numeric`, `Value::Date`, and `Value::Timestamp` carry raw proto wire strings, and `ExaType` is duplicated in `exa-zmq-protocol`. This delta introduces a `Decimal { unscaled: i128, scale: u8 }` newtype, typed temporal variants backed by `chrono::NaiveDate`/`NaiveDateTime`, and typed getters returning `Result<Option<T>, UdfError>` where SQL NULL maps to `None`. Only the scenarios below change; other udf-sdk scenarios (ABI, fingerprint, macro entry point) are unchanged.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Value and ExaType cover the v1 column types

* *GIVEN* the SDK `value` module
* *WHEN* a UDF reads or emits a column
* *THEN* `Value` MUST provide strongly typed variants for `Null`, `Int32(i32)`, `Int64(i64)`, `Double(f64)`, `Numeric(Decimal)`, `Bool(bool)`, `String(String)`, `Date(NaiveDate)`, and `Timestamp(NaiveDateTime)`, where `Numeric` carries a `Decimal { unscaled: i128, scale: u8 }` newtype and `Date`/`Timestamp` carry `chrono::NaiveDate`/`NaiveDateTime` (NOT `String`)
* *AND* the single canonical `ExaType` MUST live in the SDK `value` module and provide matching descriptors including `Numeric { precision, scale }` and `String { size }`
* *AND* `exa-zmq-protocol` MUST re-use the SDK `ExaType` rather than defining its own duplicate enum
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Decimal is constructible from string and float without precision loss

* *GIVEN* the SDK `Decimal` newtype
* *WHEN* a UDF or the runtime constructs a decimal from the proto wire form
* *THEN* `Decimal::try_from(&str)` MUST parse a signed decimal literal such as `"-1.000000000000000001"` into `unscaled` and `scale` with no precision loss for up to 38 significant digits
* *AND* `Decimal::try_from(f64)` MUST be provided for callers holding a floating-point value, returning `UdfError::Type` (or a dedicated decimal error) for non-finite inputs
* *AND* `Decimal::to_string` MUST round-trip back to the canonical decimal wire form so emit serialization is lossless
* *AND* a value whose `scale` is `0` MUST render with no decimal point
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: UdfContext exposes typed accessors and row iteration

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF inspects and reads its input
* *THEN* the trait MUST provide `next`, `reset`, `emit`, and column introspection (`column_count`, `column_name`, `column_type`, `column_index`)
* *AND* it MUST provide typed accessors `get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, `get_timestamp`, and `get_value`, each returning `Result<Option<T>, UdfError>` where a SQL NULL maps to `Ok(None)` and a matching cell maps to `Ok(Some(value))`
* *AND* `get_i64` MUST additionally accept an integral `Value::Numeric` cell (because Exasol delivers `BIGINT` as `PB_NUMERIC`), returning `UdfError::Type` only when the decimal has a non-zero fractional part
* *AND* a typed accessor invoked on a column whose `Value` variant does not match the requested type (and is not the documented `Numeric`→`i64` case) MUST return `UdfError::Type` rather than silently coercing
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: exasol_udf annotation with an unknown type fails to compile

* *GIVEN* an `#[exasol_udf]` annotation whose `input(...)` or `emits(...)` list names a Rust type the macro cannot map
* *WHEN* the crate is compiled
* *THEN* the macro MUST emit a compile error carrying the offending type's span
* *AND* the macro MUST map `i32`, `i64`, `f32`, `f64`, `bool`, `String`, and `&str`/`str` to their `ExaType` JSON names as before
* *AND* the macro MUST additionally map `Decimal`, `NaiveDate`, and `NaiveDateTime` to `Numeric`, `Date`, and `Timestamp` respectively so typed schema fields compile
<!-- /DELTA:CHANGED -->
