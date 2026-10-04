# Decisions: prune-unreachable-exatype-variants

## ADR: Prune unreachable ExaType variants

**ID:** prune-unreachable-exatype-variants
**Plan:** prune-unreachable-exatype-variants
**Status:** Accepted
**Supersedes:** extended-exasol-types-string-backed-value

### Context

The ADR `extended-exasol-types-string-backed-value` added `ExaType` variants for `Geometry`, `HashType`, `IntervalYearToMonth`, and `IntervalDayToSecond`. E2E canary tests on all three CI series (8.29.x, 2025.1.x, 2026.1.x) confirm the DB rejects these types as UDF columns before values ever reach the wire. The variants are unreachable dead code. The work was driven by issue #108 evidence, not by a plan.

### Decision

Remove `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, and `TimestampTz` from `ExaType`. Any `PB_STRING` column with an unrecognised `type_name` maps to `ExaType::String`. Any `PB_TIMESTAMP` maps to `ExaType::Timestamp` (the DB rejects `TIMESTAMP WITH LOCAL TIME ZONE` as a UDF column in both directions, so `TimestampTz` was unreachable).

### Consequences

Breaking change: downstream code matching on the removed variants will not compile. Version bumped 0.27.0 → 0.28.0; every downstream UDF must rebuild (ABI fingerprint includes the version).
