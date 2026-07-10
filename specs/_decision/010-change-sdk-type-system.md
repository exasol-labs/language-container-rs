# Decisions: change-sdk-type-system

## ADR: Numeric represented as a custom Decimal { unscaled: i128, scale: u8 } newtype

**ID:** numeric-custom-decimal-newtype
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

`Value::Numeric` must carry a lossless decimal representation for Exasol's `DECIMAL(p,s)` type, which supports up to 36 significant digits. The `rust_decimal` crate uses a 96-bit mantissa, capping precision at ~28-29 significant digits. Parsing a proto wire string through `rust_decimal` would silently lose precision on values with 29–36 significant digits. The project also has a musl-static-link constraint favouring minimal dependencies and zero data transformation on the hot path.

### Decision

Add a zero-dependency `Decimal { unscaled: i128, scale: u8 }` newtype in `exasol-udf-sdk::value` with `TryFrom<&str>`, `TryFrom<f64>`, and a lossless `Display`. `Value::Numeric` carries this `Decimal`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Custom `Decimal { i128, u8 }` newtype | ✓ Chosen — `i128` holds 38 digits, covering Exasol's full range; zero new dependencies; exact lossless round-trip; aligns with the musl-static-link and zero-transformation goals |
| `rust_decimal::Decimal` | ✗ Rejected — capped at ~28-29 significant digits (96-bit mantissa); would silently lose precision on Exasol DECIMAL values with 29–36 digits |

### Consequences

The SDK carries a bespoke decimal type. Authors who need arithmetic must either use the `unscaled`/`scale` fields directly or convert to a floating-point type (with the documented precision trade-off). The `rust_decimal` crate is not added to the workspace. Lossless wire round-trip is guaranteed for Exasol's full 36-digit DECIMAL range.

## ADR: Deduplicate ExaType by making exa-zmq-protocol depend on exasol-udf-sdk

**ID:** deduplicate-exatype-protocol-depends-on-sdk
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

`ExaType` was duplicated verbatim in both `exasol-udf-sdk` and `exa-zmq-protocol`, allowing the two copies to drift. The SDK is the author-facing home of the type model. Two options existed: add a new `exa-types` leaf crate that both depend on, or add a direct dependency edge from the protocol crate to the SDK.

### Decision

The single canonical `ExaType` lives in `exasol-udf-sdk::value`. `exa-zmq-protocol` adds a dependency edge on `exasol-udf-sdk` and re-exports that enum. The dependency graph becomes `protocol → {exa-proto, exasol-udf-sdk}` and `runtime → {protocol, exasol-udf-sdk}` — no cycle.

### Options Considered

| Option | Verdict |
|--------|---------|
| Add dependency edge `exa-zmq-protocol → exasol-udf-sdk` | ✓ Chosen — one edge addition produces no cycle; far less churn than introducing, versioning, and publishing a new crate; the SDK is already the natural home of the author-facing type model |
| Extract a new `exa-types` leaf crate | ✗ Rejected — requires creating, versioning, and publishing a new crate; significantly more churn for the same outcome |

### Consequences

`exa-zmq-protocol` gains a compile-time dependency on `exasol-udf-sdk`. The duplicate `ExaType` enum in `exa-zmq-protocol/src/meta.rs` is deleted. All downstream code uses `exasol_udf_sdk::value::ExaType` as the single type. The dependency graph remains acyclic.

## ADR: Extended Exasol types are String-backed Value payloads but distinct ExaType variants

**ID:** extended-exasol-types-string-backed-value
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

Several Exasol SQL types (`TIMESTAMP WITH LOCAL TIME ZONE`, `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND`, `GEOMETRY`, `HASHTYPE`, `CHAR`) are transmitted over the wire as a proto `STRING` block. Fully typed representations (e.g. `chrono::DateTime<FixedOffset>` for `TimestampTz`, dedicated interval/geometry structs) would require non-trivial parsing and new dependencies. A decision was needed on whether to make these types fully typed `Value` payloads or to distinguish them only at the `ExaType` level.

### Decision

Distinguish extended types at the `ExaType` level (new variants: `Char { size }`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, `TimestampTz`) refined from `type_name` at `ColumnMeta` construction time. The `Value` payload for all these types remains the wire `String`. Only `Date`, `Timestamp`, and `Numeric` become fully typed (`NaiveDate`, `NaiveDateTime`, `Decimal`).

### Options Considered

| Option | Verdict |
|--------|---------|
| Distinct `ExaType` variants, `String` wire payload | ✓ Chosen — proto block is a string and does not change; Exasol timezone and interval semantics are complex; `ExaType` gives authors the SQL-type information without conversion cost; zero allocation on the hot path |
| Fully typed `chrono::DateTime<FixedOffset>` for TimestampTz and dedicated interval/geometry structs | ✗ Rejected — complex semantics; proto block stays string anyway; rarely arithmetic-driven inside a UDF; would add non-trivial parsing and potentially new dependencies |

### Consequences

Authors receive the SQL type distinction via `ColumnMeta::typ` (`ExaType` variant) but receive the raw wire string as the `Value` payload for extended types. Extended-type arithmetic (timezone conversion, interval math) must be handled by the author. The `ExaType` refinement happens once at `ColumnMeta::from_pb`, downstream code sees a rich `ExaType` with no repeated `type_name` inspection.
