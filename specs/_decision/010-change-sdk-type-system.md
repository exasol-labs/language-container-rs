# Decisions: change-sdk-type-system

## ADR: Numeric represented as a custom Decimal { unscaled: i128, scale: u8 } newtype

**ID:** numeric-custom-decimal-newtype
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

`Value::Numeric` must carry Exasol `DECIMAL(p,s)` values of up to 36 significant digits losslessly. `rust_decimal` has a 96-bit mantissa, which holds only ~28-29 digits. The musl static link favors minimal dependencies.

### Decision

`exasol-udf-sdk::value` defines a zero-dependency `Decimal { unscaled: i128, scale: u8 }` newtype with `TryFrom<&str>`, `TryFrom<f64>` and a lossless `Display`. `Value::Numeric` carries this `Decimal`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Custom `Decimal { i128, u8 }` | ✓ Chosen, `i128` covers 38 digits, no new dependency |
| `rust_decimal::Decimal` | ✗ Loses precision beyond ~28-29 digits |

### Consequences

Authors needing arithmetic use the `unscaled`/`scale` fields or convert to a floating-point type with reduced precision. The workspace does not depend on `rust_decimal`.

## ADR: Deduplicate ExaType by making exa-zmq-protocol depend on exasol-udf-sdk

**ID:** deduplicate-exatype-protocol-depends-on-sdk
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

Two copies of `ExaType` can drift. The SDK is the author-facing home of the type model.

### Decision

`ExaType` lives only in `exasol-udf-sdk::value`. `exa-zmq-protocol` depends on `exasol-udf-sdk` and re-exports that enum. The dependency graph is `protocol → {exa-proto, exasol-udf-sdk}` and `runtime → {protocol, exasol-udf-sdk}`, with no cycle.

### Options Considered

| Option | Verdict |
|--------|---------|
| Edge `exa-zmq-protocol → exasol-udf-sdk` | ✓ Chosen, one edge, no cycle |
| New `exa-types` leaf crate | ✗ Needs a new versioned, published crate |

### Consequences

Downstream code uses `exasol_udf_sdk::value::ExaType`. `exa-zmq-protocol` has a compile-time dependency on the SDK.

## ADR: Extended Exasol types are String-backed Value payloads but distinct ExaType variants

**ID:** extended-exasol-types-string-backed-value
**Plan:** `change-sdk-type-system`
**Status:** Superseded by prune-unreachable-exatype-variants

### Context

`TIMESTAMP WITH LOCAL TIME ZONE`, `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND`, `GEOMETRY`, `HASHTYPE` and `CHAR` travel as a proto `STRING` block. Typed representations need non-trivial parsing and new dependencies.

### Decision

`ExaType` has distinct variants for these types (`Char { size }`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, `TimestampTz`), refined from `type_name` when `ColumnMeta` is constructed. Their `Value` payload is the wire `String`. Only `Date`, `Timestamp` and `Numeric` are fully typed (`NaiveDate`, `NaiveDateTime`, `Decimal`).

### Options Considered

| Option | Verdict |
|--------|---------|
| Distinct `ExaType` variants, `String` payload | ✓ Chosen, no hot-path conversion |
| Typed payloads (`chrono::DateTime<FixedOffset>`, interval and geometry structs) | ✗ Complex semantics, rarely needed in UDFs, new parsing and dependencies |

### Consequences

Authors see the SQL type in `ColumnMeta::typ` but receive the raw string. Authors handle timezone conversion and interval math themselves.
