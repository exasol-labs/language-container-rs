# Plan: change-sdk-type-system

## Summary

Replace the string-based `Value` variants in `exasol-udf-sdk` with strongly typed Rust types (`Decimal`, `NaiveDate`, `NaiveDateTime`), deduplicate `ExaType` into a single SDK-owned enum re-used by `exa-zmq-protocol`, add the full set of typed `UdfContext` getters, and extend `ExaType` with rich Exasol-specific variants inferred from `ColumnMeta::type_name`.

## Design

### Context

The SDK is the author-facing contract crate for Rust UDFs. Today three "raw" `Value` variants (`Numeric`, `Timestamp`, `Date`) carry proto wire strings, forcing every UDF author to hand-parse decimal/date strings and breaking the "no-brainer API" goal. `ExaType` is duplicated verbatim in both `exasol-udf-sdk` and `exa-zmq-protocol`, so the two can drift. `ColumnMeta::from_pb` maps only the eight proto column types and discards the SQL-level distinctions Exasol encodes in `type_name` (CHAR vs VARCHAR, TIMESTAMP vs TIMESTAMP WITH LOCAL TIME ZONE, INTERVAL/GEOMETRY/HASHTYPE on PB_STRING).

- **Goals** — typed `Value` with zero hand-parsing for authors; a single canonical `ExaType`; full Exasol type coverage surfaced via `type_name` refinement; typed getters returning `Result<Option<T>, UdfError>`; lossless, allocation-frugal wire round-trip; no new heavyweight runtime dependency.
- **Non-Goals** — changing the proto/wire format (the proto block stays string for numeric/temporal/extended types); adding timezone-aware arithmetic; supporting Exasol types beyond the documented set; rewriting the connect-back arrow decode beyond the `Value` shape change.

### Decision

`Value::Numeric` carries a custom `Decimal { unscaled: i128, scale: u8 }` newtype. `Value::Date`/`Value::Timestamp` carry `chrono::NaiveDate`/`NaiveDateTime`. The canonical `ExaType` lives in `exasol-udf-sdk::value` and gains extended variants; `exa-zmq-protocol` adds a dependency on `exasol-udf-sdk` and re-exports that enum. Typed getters are strict (no silent coercion) except the documented `Numeric`→`i64` integral case.

#### Architecture

```
exasol-udf-sdk (value: Value, Decimal, ExaType)  ◀── canonical types
       ▲                         ▲
       │                         │ (NEW dep: protocol → sdk)
exasol-udf-macros          exa-zmq-protocol (ColumnMeta re-exports ExaType,
                                              from_pb refines via type_name)
                                  ▲
                           exa-udf-runtime (rowset encode/decode of typed Value)
```

No dependency cycle: `exa-zmq-protocol → {exa-proto, exasol-udf-sdk}`; `exa-udf-runtime → {exa-zmq-protocol, exasol-udf-sdk}`.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Newtype over `i128`+`u8` | `Decimal` in `value.rs` | 38-digit precision (Exasol DECIMAL max), zero new dep, exact round-trip |
| Single source of truth | `ExaType` in SDK, re-export in protocol | Eliminates the duplicate enum that could drift |
| `type_name` refinement at construction | `ColumnMeta::from_pb` | Extended types resolved once, downstream code sees a rich `ExaType` |
| Strict typed getters with one documented exception | `UdfContext` getters | Predictable API; the `Numeric`→`i64` case reflects Exasol delivering BIGINT as PB_NUMERIC |
| Encode by declared column type, not `Value` variant | `EmitBuffer::to_proto` | Preserves existing invariant that the EMITS schema dictates the proto block |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Custom `Decimal { i128, u8 }` | `rust_decimal::Decimal` | `rust_decimal` caps at ~28-29 significant digits (96-bit mantissa); Exasol DECIMAL allows 36, so wire strings would lose precision. `i128` holds 38 digits. Custom type adds zero deps, aligning with the musl-static-link and zero-transformation goals. |
| Protocol depends on SDK | New `exa-types` leaf crate | Adding one edge is less churn than a new crate to version/publish; no cycle results. The SDK is already the natural home of the author-facing type model. |
| Extended types as `String`-backed `Value` (TimestampTz, Interval*, Geometry, HashType, Char) | Typed `chrono::DateTime<FixedOffset>` / interval structs | Wire form is a string and the proto block does not change; Exasol tz/interval semantics are complex. `ExaType` still distinguishes them so authors know the SQL type; zero conversion on the hot path. |
| `Value::Date`/`Timestamp` typed (NaiveDate/NaiveDateTime) | Keep as `String` | The design doc mandates `get_date`/`get_timestamp`; typed variants make those getters trivial and give authors real date objects. Parse happens once at decode. |
| Strict getters (no silent cross-type cast) | Auto-cast everywhere | Silent coercion hides schema bugs. The single `Numeric`→`i64` exception is required by Exasol's BIGINT-as-PB_NUMERIC reality and is explicitly documented. |
| `chrono = "0.4"` (default-features, no `clock`/`std` trimming yet) | No chrono / hand-rolled date math | Standard, musl-compatible, latest stable `0.4`; date/time correctness is not worth hand-rolling. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-sdk | CHANGED | `specs/_plans/change-sdk-type-system/sdk/udf-sdk/spec.md` |
| protocol/wire-protocol | CHANGED | `specs/_plans/change-sdk-type-system/protocol/wire-protocol/spec.md` |

## Dependencies

- Add `chrono = "0.4"` to `[workspace.dependencies]` and to `exasol-udf-sdk` (and `exa-zmq-protocol`/`exa-udf-runtime` as needed transitively).
- Add `exasol-udf-sdk` as a dependency of `exa-zmq-protocol`.
- No `rust_decimal` dependency (custom `Decimal` instead).

## Migration

| Current | New |
|---------|-----|
| `Value::Numeric(String)` | `Value::Numeric(Decimal)` |
| `Value::Date(String)` | `Value::Date(NaiveDate)` |
| `Value::Timestamp(String)` | `Value::Timestamp(NaiveDateTime)` |
| `ExaType` (unit enum, two copies) | `ExaType` (SDK-owned, extended variants with fields) |
| `exa_zmq_protocol::ExaType` | re-export of `exasol_udf_sdk::value::ExaType` |
| UDFs parsing `Value::Numeric(s)` by hand | `ctx.get_i64(0)?` / `ctx.get_decimal(0)?` |

## Implementation Tasks

1. Add `chrono = "0.4"` to `[workspace.dependencies]`; add the `exasol-udf-sdk` dependency edge to `exa-zmq-protocol/Cargo.toml` and `chrono` to the crates that need it.
2. Introduce the `Decimal { unscaled: i128, scale: u8 }` newtype in `exasol-udf-sdk/src/value.rs` with `TryFrom<&str>`, `TryFrom<f64>`, and `Display` (lossless wire round-trip). [expert]
3. Rework the `Value` enum: `Numeric(Decimal)`, `Date(NaiveDate)`, `Timestamp(NaiveDateTime)`; add the canonical extended `ExaType` (`Char { size }`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, `TimestampTz`, plus existing variants with fields). [expert]
4. Remove the duplicate `ExaType` from `exa-zmq-protocol/src/meta.rs`; re-export the SDK enum and update `ColumnMeta`. [expert]
5. Implement `type_name` refinement in `ColumnMeta::from_pb` (CHAR/VARCHAR, GEOMETRY/HASHTYPE/INTERVAL on PB_STRING, TIMESTAMP WITH LOCAL TIME ZONE on PB_TIMESTAMP) and the inverse mapping in `ColumnMeta::to_pb`.
6. Add the typed getters (`get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, `get_timestamp`, `get_value`) plus `reset`/column-introspection methods to `UdfContext`, with NULL→`None` and strict type checks (documented `Numeric`→`i64` exception).
7. Update `exa-udf-runtime/src/rowset.rs` decode/encode to materialize and serialize the typed `Value` variants losslessly across all type blocks (Decimal, Date, Timestamp, extended string-backed types). [expert]
8. Update `exasol-udf-sdk/src/connect_back.rs` (`cell_to_value`) to produce typed `Value::Numeric(Decimal)`/`Value::Date(NaiveDate)`/`Value::Timestamp(NaiveDateTime)` from arrow cells. [expert]
9. Update `exasol-udf-macros/src/lib.rs` `rust_type_to_exatype` to additionally map `Decimal`, `NaiveDate`, `NaiveDateTime`.
10. Update all `test-udfs/*` that read or emit `Value` (scalar-double, set-filter, json-parse, annotated-*, connect-back-*) to the typed API.
11. Add the "## Exasol data type mapping" section to `language-container-rs/CLAUDE.md` (8 proto types, SQL types, type_name disambiguation, ExaType variant).
12. Update unit tests in `value.rs`, `rowset.rs`, `meta.rs`, `context.rs`, `connect_back.rs` to cover the new types; run fmt + clippy + tests.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A (foundation) | Task 1 |
| Group B (SDK core) | Task 2, Task 3 |
| Group C (consumers) | Task 4, Task 5, Task 6 |
| Group D (downstream) | Task 7, Task 8, Task 9 |
| Group E (callers + docs) | Task 10, Task 11 |
| Group F (verify) | Task 12 |

Sequential dependencies:
- Group A → Group B (deps must exist first)
- Group B → Group C (protocol/getters need the new `Value`/`ExaType`)
- Group C → Group D (runtime/macro/connect-back need the deduped `ExaType` + getters)
- Group D → Group E (test-udfs need the full typed API)
- Group E → Group F (verify last)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Enum | `exa-zmq-protocol/src/meta.rs::ExaType` | Replaced by re-export of `exasol_udf_sdk::value::ExaType` |
| Logic | `rowset.rs::value_to_block_string` string passthrough for `Numeric`/`Date`/`Timestamp` | Those variants are no longer `String`; replaced by `Decimal::to_string` / chrono formatting |
| UDF code | hand-parsing of `Value::Numeric(s)` in `test-udfs/scalar-double` and others | Replaced by typed getters / typed `Decimal` arithmetic |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Value and ExaType cover the v1 column types | Unit | `crates/exasol-udf-sdk/src/value.rs` | `value_exatype_typed_variants` |
| Decimal is constructible from string and float without precision loss | Unit | `crates/exasol-udf-sdk/src/value.rs` | `decimal_from_str_and_f64_roundtrip` |
| UdfContext exposes typed accessors and row iteration | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `bridge_typed_getters_return_typed_options` |
| exasol_udf annotation with an unknown type fails to compile | Unit | `crates/exasol-udf-macros` (trybuild/compile test) | `macro_maps_decimal_date_timestamp` |
| Metadata maps proto column types to ColumnMeta | Unit | `crates/exa-zmq-protocol/src/meta.rs` | `from_pb_uses_sdk_exatype` |
| ColumnMeta refines extended Exasol types from type_name | Unit | `crates/exa-zmq-protocol/src/meta.rs` | `from_pb_refines_extended_types_via_type_name` |
| Extended ExaType variants round-trip back to proto column types | Unit | `crates/exa-zmq-protocol/src/meta.rs` | `extended_exatype_roundtrips_to_pb` |
| End-to-end typed scalar (BIGINT/DECIMAL) | Integration | `crates/it/tests/db_roundtrip.rs` | `scalar_double_returns_42` (existing, updated for typed API) |
| End-to-end typed set/EMITS | Integration | `crates/it/tests/db_roundtrip.rs` | `set_filter_emits_positive_only` (existing, updated) |

Notes:
- The `Decimal` round-trip and the `type_name` refinement are pure computation with no I/O — unit tests are the correct form per `specs/mission.md`.
- The DB round-trip integration tests (existing, in `it`) prove the typed `Value` survives the full ZMQ + DB path; they are updated, not replaced.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-sdk | `cargo test -p exasol-udf-sdk` | `value` and `decimal` tests pass; 0 failures |
| protocol/wire-protocol | `cargo test -p exa-zmq-protocol` | `from_pb`/`to_pb` extended-type tests pass; 0 failures |
| typed runtime | `cargo +1.91 test -p exa-udf-runtime` | rowset encode/decode of typed `Value` passes |
| end-to-end | `make test-e2e` (or `cargo +1.91 test -p it --features integration`) | scalar-double returns 42; set-filter emits positives; 0 failures against local Exasol Docker |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Integration | `cargo +1.91 test -p it --features integration` | 0 failures (fails, not skips, if Exasol Docker unavailable) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
