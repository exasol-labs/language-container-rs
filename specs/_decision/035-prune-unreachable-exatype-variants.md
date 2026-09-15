# Decision: prune unreachable ExaType variants

**ID:** prune-unreachable-exatype-variants
**Plan:** (none — driven by issue #108 evidence)
**Status:** Accepted
**Supersedes:** D010 `extended-exasol-types-string-backed-value` (the choice to model `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond` as `ExaType` variants)

## Context

D010 added `ExaType` variants for `Geometry`, `HashType`, `IntervalYearToMonth`, and `IntervalDayToSecond`. E2E canary tests on all three CI series (8.29.x, 2025.1.x, 2026.1.x) confirm the DB rejects these types as UDF columns before values ever reach the wire. The variants are unreachable dead code.

## Decision

Remove `Geometry`, `HashType`, `IntervalYearToMonth`, and `IntervalDayToSecond` from `ExaType`. Any `PB_STRING` column with a `type_name` that is not `CHAR…` or `VARCHAR…` maps to `ExaType::String`. `TimestampTz` stays (ingest-only); the SDK rejects it as an output column at emit validation time.

## Consequences

Breaking change: downstream code matching on the removed variants will not compile. Version bumped 0.27.0 → 0.28.0; every downstream UDF must rebuild (ABI fingerprint includes the version).
