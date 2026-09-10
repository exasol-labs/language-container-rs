# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the row-based emit surface — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`; the ABI vtable and `emit-arrow` feature boundary are specified in `sdk/udf-abi`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. Output is produced two ways selected by the UDF function's return type: an EMITS function returns `Result<(), UdfError>` and pushes rows through `ctx.emit()`; a RETURNS function returns `Result<Option<T>, UdfError>` and its value becomes the single output row.

`UdfContext` exposes plain handshake metadata to UDF code. Beyond the typed column accessors it provides `memory_limit()` and the `exascript_info` identity/origin accessors (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`), each sourced from `UdfMeta`; these are defaulted accessors (not feature-gated) so existing implementations keep compiling, overridden by the host context bridge to return the live value.

A UDF author needs a `UdfContext` to unit-test a UDF function without a live host. The SDK therefore ships that double itself, behind the non-default `test-support` cargo feature, so no author and no in-repo fixture hand-writes an `impl UdfContext`. The feature adds items only. It declares no dependency and enables no other feature, so a dependent crate's featureless test configuration keeps its meaning.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: UdfContext exposes typed accessors and row iteration

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF inspects and reads its input
* *THEN* the trait MUST provide `num_columns`, `get`, `emit`, and `next` as required methods
* *AND* it MUST provide typed accessors `get_value`, `get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, and `get_timestamp`, each returning `Result<Option<T>, UdfError>` where a SQL NULL maps to `Ok(None)` and a matching cell maps to `Ok(Some(…))`
* *AND* `get_i64` MUST additionally accept an integral `Numeric` cell (because Exasol delivers `BIGINT` as `PB_NUMERIC`), returning `Err(UdfError::Type)` only when the decimal has a non-zero fractional part
* *AND* a typed accessor invoked on a column whose `Value` variant does not match the requested type (and is not the documented `Numeric`→`i64` case) MUST return `Err(UdfError::Type)` rather than silently coercing
* *AND* the trait MUST additionally provide `emit_owned(&mut self, values: Vec<Value>) -> Result<(), UdfError>`, whose default implementation forwards to `emit(&values)`, so every existing `impl UdfContext` keeps compiling
* *AND* `emit_owned` MUST move each `Value` into the host's emit buffer, whereas `emit` clones
* *AND* both methods MUST enforce the same RETURNS ban: a call in `output_iter = ExactlyOnce` context MUST return `Err(UdfError)`
<!-- /DELTA:CHANGED -->
</content>
