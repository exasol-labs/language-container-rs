# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the row-based emit surface — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`; the ABI vtable and `emit-arrow` feature boundary are specified in `sdk/udf-abi`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. Output is produced two ways selected by the UDF function's return type: an EMITS function returns `Result<(), UdfError>` and pushes rows through `ctx.emit()`; a RETURNS function returns `Result<Option<T>, UdfError>` and its value becomes the single output row.

`UdfContext` exposes plain handshake metadata to UDF code. Beyond the typed column accessors it provides `memory_limit()` and the `exascript_info` identity/origin accessors (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`), each sourced from `UdfMeta`; these are defaulted accessors (not feature-gated) so existing implementations keep compiling, overridden by the host context bridge to return the live value.

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

### Scenario: UdfContext exposes handshake identity and origin metadata

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF queries the handshake metadata the database delivered in `exascript_info`
* *THEN* the trait MUST provide the numeric accessors `session_id(&self) -> u64`, `statement_id(&self) -> u32`, `node_id(&self) -> u32`, `node_count(&self) -> u32`, and `vm_id(&self) -> u64`, the owned-string accessors `database_name`, `database_version`, `script_name`, and `script_schema` (returning owned values, not borrows, because they cross the `.so` vtable boundary), and the optional accessors `current_user`, `current_schema`, and `scope_user` returning `Option<String>` to mirror the `optional` proto fields, each sourced from the corresponding `UdfMeta` field
* *AND* every accessor MUST be a provided (defaulted) trait method, mirroring `memory_limit()`, so existing `UdfContext` implementations continue to compile without supplying it
* *AND* the numeric accessors MUST default to `0`, the owned-string accessors MUST default to the empty string, and the optional accessors MUST default to `None`, each denoting "not reported"
* *AND* none of the accessors MAY be gated behind the `connect-back` feature, because handshake metadata is plain DB-supplied context rather than a connect-back capability

### Scenario: UdfContext exposes the per-instance memory limit

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF queries the resident-memory limit the database allotted to its VM instance
* *THEN* the trait MUST provide a `memory_limit(&self) -> u64` accessor returning the limit in bytes, sourced from `UdfMeta::maximal_memory_limit`
* *AND* the accessor MUST be a provided (defaulted) trait method returning `0` (denoting "no limit reported") so existing `UdfContext` implementations continue to compile without supplying it, mirroring how the SDK keeps the data-access surface backward compatible
* *AND* the accessor MUST NOT be gated behind the `connect-back` feature, because the limit is plain handshake metadata rather than a connect-back capability
* *AND* the accessor MUST follow the same defaulted-accessor pattern as the identity and origin metadata accessors, with the host context bridge overriding the default to return the exact byte value carried on `UdfMeta::maximal_memory_limit`

### Scenario: A value-returning UDF function selects the RETURNS output shape

* *GIVEN* a `#[exasol_udf]` function whose signature is `Result<Option<T>, UdfError>` for a `T` the SDK can convert to `Value`
* *WHEN* the function returns `Some(v)` or `None`
* *THEN* the SDK MUST provide an `IntoValue`-style conversion mapping each supported author type — `i64`, `i32`, `f64`, `bool`, `String`, `&str`, `Decimal`, `NaiveDate`, `NaiveDateTime`, and `Value` itself — to the matching `Value` variant
* *AND* `None` MUST convert to `Value::Null`, so a RETURNS function expresses SQL NULL as `Ok(None)`
* *AND* the unit form `Result<(), UdfError>` MUST remain the EMITS shape, so `UdfRun::run` and existing EMITS UDFs that produce output through `ctx.emit()` compile and behave unchanged

### Scenario: UdfContext exposes a set_return channel for RETURNS output

* *GIVEN* the `UdfContext` trait
* *WHEN* the framework delivers a RETURNS function's returned value
* *THEN* the trait MUST provide `set_return(&mut self, value: Option<Value>) -> Result<(), UdfError>` used by the generated RETURNS shim to hand the single output row to the host, distinct from `emit`
* *AND* `set_return` MUST be a provided (defaulted) trait method whose default returns `UdfError::Unimplemented`, so existing `UdfContext` implementations continue to compile without supplying it
* *AND* `value = None` MUST denote a single SQL NULL output row and `value = Some(v)` a single output row carrying `v`
