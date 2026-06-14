# Decision Log: change-sdk-type-system

Date: 2026-06-14

## Interview

**Q:** What does "optimize" mean for this SDK change?
**A:** All of: (1) replace string-based `Value` variants with typed Rust types, (2) add typed `UdfContext` getters (`get_i64`, `get_f64`, etc.), (3) deduplicate `ExaType` across crates, (4) add rich `ExaType` variants for extended Exasol types. Emphasis: "as fast as possible; focus on developer experience; make this a no-brainer; all types must be supported; zero copy or type transformation if avoidable."

**Q:** How should DECIMAL/NUMERIC be represented?
**A:** No specific option chosen, but it must be creatable from a string and from a floating-point number when available. (Design doc had specified `Value::Numeric(i128, u8)`.) Planner to choose between a custom `Decimal { unscaled: i128, scale: u8 }` newtype or the `rust_decimal` crate.

**Q:** How should extended Exasol types not present in the proto be handled?
**A:** As distinct `ExaType` variants: `IntervalYearToMonth`, `IntervalDayToSecond`, `Geometry`, `HashType`, `TimestampTz`, and `Char` (distinct from `Varchar`/`String`). Inferred at `ColumnMeta` construction time by checking both the proto `column_type` AND the `type_name` string. Wire serialization stays `String` (the proto block does not change).

**Q:** What CLAUDE.md rule should be added?
**A:** A proto-to-Exasol mapping table documenting the 8 proto column types and how they map, including which SQL types collapse to `PB_STRING` and are identified via `type_name`.

## Design Decisions

### [1] Numeric represented as a custom `Decimal { unscaled: i128, scale: u8 }` newtype

- **Decision:** Add a zero-dependency `Decimal` newtype in `exasol-udf-sdk::value` with `TryFrom<&str>`, `TryFrom<f64>`, and a lossless `Display`. `Value::Numeric` carries this `Decimal`.
- **Alternatives:** `rust_decimal::Decimal` (ergonomic, has `from_f64_retain`/`TryFrom<f64>`).
- **Rationale:** `rust_decimal` is limited to a 96-bit mantissa (~28-29 significant digits). Exasol `DECIMAL` supports up to 36 digits, so parsing the proto wire string through `rust_decimal` would silently lose precision. An `i128` unscaled mantissa holds 38 digits, covering Exasol's full range. The custom type also adds no dependency, which matches the "zero transformation if avoidable" goal and the project's musl-static-link constraint.
- **Promotes to ADR:** yes

### [2] Deduplicate `ExaType` by making `exa-zmq-protocol` depend on `exasol-udf-sdk`

- **Decision:** The single canonical `ExaType` lives in `exasol-udf-sdk::value`; `exa-zmq-protocol` adds a dependency edge on the SDK and re-exports that enum.
- **Alternatives:** Extract a new `exa-types` leaf crate that both depend on.
- **Rationale:** Adding one dependency edge produces no cycle (`protocol → {proto, sdk}`, `runtime → {protocol, sdk}`) and is far less churn than introducing, versioning, and publishing a new crate. The SDK is already the author-facing home of the type model.
- **Promotes to ADR:** yes

### [3] Extended types (TimestampTz, Interval*, Geometry, HashType, Char) are String-backed `Value`s but distinct `ExaType`s

- **Decision:** Distinguish these at the `ExaType` level (refined from `type_name` at `ColumnMeta` construction), but keep their `Value` payload as the wire `String`. Only `Date`/`Timestamp`/`Numeric` become fully typed (`NaiveDate`/`NaiveDateTime`/`Decimal`).
- **Alternatives:** Fully typed `chrono::DateTime<FixedOffset>` for TimestampTz and dedicated interval/geometry structs.
- **Rationale:** The proto block for these is a string and does not change; Exasol timezone and interval semantics are complex and rarely arithmetic-driven inside a UDF. Distinguishing the `ExaType` gives authors the SQL-type information they need while keeping the hot path allocation/conversion-free.
- **Promotes to ADR:** yes

### [4] Typed getters are strict, with one documented `Numeric`→`i64` exception

- **Decision:** Each `get_*` returns `Result<Option<T>, UdfError>`, NULL maps to `Ok(None)`, and a type mismatch returns `UdfError::Type`. The sole exception: `get_i64` accepts an integral `Value::Numeric`, because Exasol delivers `BIGINT` as `PB_NUMERIC`.
- **Alternatives:** Auto-cast across numeric types everywhere.
- **Rationale:** Silent coercion hides schema mistakes. The `Numeric`→`i64` allowance is an unavoidable consequence of Exasol's wire behavior and is documented at the call site and in the spec.
- **Promotes to ADR:** no

### [5] Add `chrono = "0.4"` to the workspace; no `rust_decimal`

- **Decision:** `chrono` `0.4` (latest stable) is added to `[workspace.dependencies]`. No `rust_decimal`.
- **Alternatives:** Hand-rolled date math; `rust_decimal` for numerics.
- **Rationale:** `chrono` is the standard, musl-compatible date/time crate; reimplementing date parsing is not worth the risk. `rust_decimal` is rejected per decision [1].
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
