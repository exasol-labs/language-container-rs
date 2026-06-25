# Feature: connect-back

Defines the connect-back surface of the author-facing SDK — the `ConnectionObject` credential struct, the `ExaConnection` trait, and the `UdfContext` connect-back methods. This delta removes the unsafe `query_arrow` (issue #26) and makes the connect-back types compile unconditionally so the `UdfContext` vtable can be feature-independent (issue #31).

## Background

The connect-back surface was gated behind a `connect-back` SDK feature, and `ExaConnection` exposed `query_arrow` returning `Vec<arrow::record_batch::RecordBatch>`. That signature is unsafe across the `.so` boundary: a UDF `.so` and the host each link their own static `arrow`, so downcasts on those arrays silently return `None` (mismatched `TypeId`/vtables) — wrong values, no error (issue #26).

Removing `query_arrow` makes `ExaConnection` **arrow-free**, which lets the entire `connect_back` module (and `ConnectionObject`/`ExaConnection`) compile **unconditionally** — no SDK `connect-back` feature. That is the prerequisite for issue #31's fix: with the connect-back types always present, `UdfContext` can declare its `connection`/`connect_back` methods unconditionally, giving a feature-independent trait-object vtable. Arrow remains the host's internal transport (exarrow-rs → batches → `Vec<Value>`); UDFs only ever receive `Vec<Value>`.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: ExaConnection trait is arrow-free and always compiled

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* the `connect_back` module is referenced
* *THEN* the `connect_back` module and the public `ConnectionObject` and `ExaConnection` items MUST compile **unconditionally** (no `#[cfg(feature = "connect-back")]` gate) and the crate MUST NOT define a `connect-back` cargo feature, because the trait no longer references any `arrow` type and so needs no optional dependency
* *AND* `ExaConnection` MUST expose `query`, `query_for_each`, `execute`, `execute_batch`, `begin`, `commit`, and `rollback`, all returning `Result<_, UdfError>`, none of which reference any `arrow` or `exarrow-rs` type in their public signature
* *AND* the trait MUST NOT declare `query_arrow` (or any method returning `Vec<RecordBatch>` / `Arc<dyn Array>` across the `.so` boundary), because that `repr(Rust)` Arrow type is not ABI-stable across two independently linked static `arrow` copies (issue #26)
* *AND* `query_for_each` MUST be a required method taking the SQL plus a row callback `F: FnMut(Vec<Value>) -> Result<(), UdfError>`, and `query` MUST default to calling `query_for_each` and collecting into `Vec<Vec<Value>>`, so both share one code path and neither depends on a boundary-crossing Arrow type
* *AND* `execute_batch` (accepting `sql: &str`, `rows: &[Vec<Value>]`), `begin`, `commit`, and `rollback` MUST each have a default implementation returning `UdfError::Unimplemented`, so mocks and connections that do not support them continue to compile unchanged
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: ConnectionObject is a public connect-back SDK type

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* the `connect_back` module is referenced
* *THEN* it MUST expose a public `ConnectionObject` struct with public `kind`, `address`, `user`, and `password` fields, available unconditionally (no feature gate)
* *AND* the `ConnectionObject` type MUST NOT reference any transport-layer type (it MUST NOT re-export or alias exarrow-rs internals)
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: query_arrow is removed from the cross-boundary trait surface

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* a UDF or a mock references the `ExaConnection` trait
* *THEN* the trait MUST NOT expose `query_arrow` (issue #26 footgun) nor any method handing back a `repr(Rust)` Arrow type across the `.so` boundary
* *AND* tests and mocks that previously implemented `query_arrow` MUST instead implement `query_for_each` (now required) to supply their rows, so the trait stays object-safe and the `Value`-based default (`query`) keeps working
* *AND* the SDK MUST document that connect-back results are delivered only as `Vec<Value>` rows; Arrow is the host's internal transport and never crosses to UDF code
<!-- /DELTA:NEW -->
