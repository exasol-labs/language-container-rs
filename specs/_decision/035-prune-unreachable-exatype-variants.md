# Decisions: prune-unreachable-exatype-variants

## ADR: Prune unreachable ExaType variants

**ID:** prune-unreachable-exatype-variants
**Plan:** prune-unreachable-exatype-variants
**Status:** Accepted
**Supersedes:** extended-exasol-types-string-backed-value

### Context

The DB rejects `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond` and `TIMESTAMP WITH LOCAL TIME ZONE` as UDF columns before values reach the wire. E2E canaries confirm this on the 8.29.x, 2025.1.x and 2026.1.x series.

### Decision

`ExaType` has no `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond` or `TimestampTz` variant. A `PB_STRING` column with an unrecognised `type_name` maps to `ExaType::String`. A `PB_TIMESTAMP` column maps to `ExaType::Timestamp`.

### Consequences

Downstream code matching on removed variants does not compile. The ABI fingerprint includes the version, so downstream UDFs rebuild.
