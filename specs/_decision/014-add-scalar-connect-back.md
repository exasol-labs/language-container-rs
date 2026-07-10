# Decisions: add-scalar-connect-back

## ADR: Connect-back is fully supported in SCALAR scripts — the shared run loop and the loopback-address fix (ADR-029) make the "never SCALAR" rule obsolete

**ID:** connect-back-fully-supported-in-scalar-scripts
**Plan:** `add-scalar-connect-back`
**Status:** Accepted

### Context

The project CLAUDE.md contained the rule: "Use `SET SCRIPT ... EMITS (...)` for any connect-back UDF; **never** `SCALAR` (SCALAR → SIGABRT mid-execution)." This rule was written as a precaution after the historical SIGABRT traced to `connect_back_sql_address()` returning `localhost:8563`, which routes to Exasol's internal CoreDB proxy and links the new session to the SQL worker (Part:40), causing a SIGABRT within seconds. That root cause was fixed in ADR-029 by switching to `<container-eth0-ip>:8563` via `getifaddrs`. The "never SCALAR" rule was never re-evaluated after ADR-029. A spike on 2026-06-17 proved empirically: (1) Python3 SCALAR connect-back returned `42` with no crash; (2) Rust SCALAR connect-back (`connect-back-scalar` crate, `RUST SCALAR SCRIPT connect_back_scalar() RETURNS BIGINT`) returned `42` with all 15 `db_roundtrip` integration scenarios passing. Code inspection of `crates/exa-udf-runtime/src/dispatch.rs` confirmed that scalar (`ExactlyOnce`) and set (`Multiple`) UDFs share one identical run loop — connect-back (MT_IMPORT exchange, external session) is transport-layer behaviour, not UDF-type-layer behaviour.

### Decision

Connect-back is a first-class supported capability for both `SCALAR` and `SET/EMITS` Rust scripts. The "never SCALAR" restriction is removed from CLAUDE.md and replaced with a positive statement: both script types support connect-back; the address rule (`cluster_ip()`, never loopback) and the transaction-conflict rule remain unchanged and apply to both. No runtime code change was required — the dispatch path was already correct. ADR-040 records the proof chain so this question cannot be re-litigated from git history alone.

### Options Considered

| Option | Verdict |
|--------|---------|
| No runtime code change — SCALAR connect-back works as-is; relax the CLAUDE.md rule | ✓ Chosen — empirically verified; zero-change to runtime; removes a stale prohibition that would cause authors to write less capable UDFs unnecessarily |
| Add a scalar-specific connect-back verification step or fast-path | ✗ Rejected — unnecessary complexity; the run loop is already shared; the spike produced no crash |
| Keep the "never SCALAR" rule with a clarifying footnote | ✗ Rejected — a rule that contradicts itself with a footnote is noise; the rule must be authoritative |
| No ADR — treat as a minor docs cleanup | ✗ Rejected — without a permanent record, the restriction will inevitably be re-introduced by a future planner seeing the old commit history without the spike output |

### Consequences

SCALAR connect-back UDFs are a supported pattern. Authors may choose `SCALAR SCRIPT ... RETURNS ...` or `SET SCRIPT ... EMITS (...)` based solely on UDF logic, not on a connect-back restriction. The address rule (`cluster_ip()`, never `127.0.0.1`) and the `std::process::exit(0)` lifecycle rule apply equally to both script types. The `connect-back-scalar` crate serves as the canonical example and integration test fixture for the scalar connect-back path.
