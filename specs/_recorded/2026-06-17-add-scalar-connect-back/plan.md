# Plan: add-scalar-connect-back

## Summary

Document and canonically record that connect-back works from Rust `SCALAR` scripts — verified live on 2026-06-17 with no runtime code change required. The plan's primary deliverables are spec deltas formalising the new scenario, a `CLAUDE.md` rule relaxation removing the stale "never SCALAR" restriction, and a decision-log ADR capturing the root cause and empirical proof.

## Design

### Context

The project `CLAUDE.md` contained the rule: "Use `SET SCRIPT ... EMITS (...)` for any connect-back UDF; **never** `SCALAR` (SCALAR → SIGABRT mid-execution)." This rule was written as a precaution after the historical SIGABRT traced to `connect_back_sql_address()` returning `localhost:8563` (fixed in plan `fix-it-matrix-connect-back-address`, ADR-029). The rule was never re-evaluated after that fix.

A spike on 2026-06-17 demonstrated empirically:

1. **Python3 SCALAR connect-back**: a `PYTHON3 SCALAR SCRIPT` connecting to the container eth0 IP via PyExasol returned `42` — no crash. SCALAR connect-back is not an Exasol engine restriction.
2. **Rust SCALAR connect-back**: new crate `test-udfs/connect-back-scalar` registered as `RUST SCALAR SCRIPT connect_back_scalar() RETURNS BIGINT`, uploaded to BucketFS, invoked via `SELECT TO_CHAR(connect_back_scalar())` — returned `42`. All 15 `db_roundtrip` integration scenarios passed.

A code-level inspection of `crates/exa-udf-runtime/src/dispatch.rs` confirms: scalar (`ExactlyOnce`) and set (`Multiple`) UDFs share one identical run loop. There is no scalar-specific connect-back path or guard. The root cause of the historical SIGABRT was the loopback address, not the UDF type.

- **Goals**
  - Spec deltas formalising SCALAR connect-back as a supported, tested capability.
  - Relaxation of the stale "never SCALAR" `CLAUDE.md` rule to reflect verified reality.
  - ADR-040 recording the empirical proof and the root-cause / resolution chain.
  - Verification that the already-implemented code, test, and CI wiring are present and complete.
- **Non-Goals**
  - Any runtime code change — the spike confirmed none is required.
  - Changes to the SET connect-back path or existing connect-back scenarios.
  - Removal of the still-valid address rule (`cluster_ip()`, never loopback).

### Decision

No architectural change. The plan records verified reality: scalar and set connect-back share one run loop; the "never SCALAR" rule was a stale precaution. Spec deltas are added to `integration/connect-back`, `examples/test-udfs`, and `runtime/dispatch-run-loop` making SCALAR connect-back an explicit first-class supported scenario.

#### Architecture

```
exaudfclient (binary)  →  std::process::exit(0) in main()
  └── exa-udf-runtime dispatch.rs
        ExactlyOnce (SCALAR)  ─┐
        Multiple     (SET)    ─┤── identical run loop — connect-back available in both
                               │
                         MT_IMPORT exchange (ZMQ idle during UDF run)
                               │
                         exarrow-rs external-client session → Exasol :8563
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Shared run loop | `dispatch.rs` | SCALAR/SET differ only in `iter_type`; connect-back is transport-layer, not UDF-type-layer |
| `std::process::exit(0)` | `exaudfclient/src/main.rs` | Prevents join delay on Tokio runtime for both scalar and set paths |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| No runtime code change | Add a scalar-specific connect-back fast-path | Not needed; dispatch is already shared |
| Relax "never SCALAR" rule in CLAUDE.md | Keep rule, add footnote | The restriction was factually wrong post-ADR-029; keeping it would mislead future authors |
| ADR-040 documents the empirical proof | No ADR (minor docs change) | Future planners must not re-litigate this; the proof chain (Python spike + Rust IT) belongs in the permanent record |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| integration/connect-back | CHANGED | `specs/_plans/add-scalar-connect-back/integration/connect-back/spec.md` |
| examples/test-udfs | CHANGED | `specs/_plans/add-scalar-connect-back/examples/test-udfs/spec.md` |
| runtime/dispatch-run-loop | CHANGED | `specs/_plans/add-scalar-connect-back/runtime/dispatch-run-loop/spec.md` |

## Dependencies

- `exasol/docker-db` images `2025.1.11`, `2025.2.1`, `2026.1.0` (already used by the IT matrix).
- `connect-back-scalar` crate: already implemented and wired to CI (no new dependency).

## Implementation Tasks

All code, test, and CI wiring is already implemented and verified green. Tasks are verification-and-record steps.

### Group A — Verify existing implementation (can run concurrently)

- [ ] 1.1 Confirm `test-udfs/connect-back-scalar/` crate (Cargo.toml + src/lib.rs) is present in the workspace and the root `Cargo.toml` includes it as a member.
- [ ] 1.2 Confirm `crates/it/tests/db_roundtrip.rs` contains `connect_back_scalar_queries_and_returns` function and `CB_SCALAR_LIB` constant, and the scenario is called from the test harness entry-point.
- [ ] 1.3 Confirm `scripts/ci-it-local.sh` includes `-p connect-back-scalar` in the UDF build list.
- [ ] 1.4 Confirm `.github/workflows/ci.yml` includes `connect-back-scalar` in its build step.

### Group B — Documentation updates (can run concurrently with Group A)

- [ ] 1.5 Update `language-container-rs/CLAUDE.md` Connect-back section: remove "never SCALAR (SCALAR → SIGABRT mid-execution)" restriction; state that both SCALAR and SET scripts support connect-back; preserve the still-valid address rule (cluster_ip, never loopback), the write-back transaction-conflict rule, and the `std::process::exit(0)` lifecycle rule.
- [ ] 1.6 Update `docs/writing-a-udf.md`: remove the stale "always use `SET SCRIPT ... EMITS (...)` for connect-back UDFs / `SCALAR SCRIPT ... RETURNS ...` crashes the SQL worker" warning (line 245); add a brief accurate note that both SCALAR and SET scripts support connect-back — pick whichever UDF type fits the logic — and that the address rules (`cluster_ip()`, never loopback) and transaction-conflict rules are unchanged; clarify the "drain all input rows before opening the connect-back session" advice (lines 298, 312) as SET-specific (SCALAR starts with the row pre-loaded; the shared-thread/ZMQ caveat still applies to both).

### Group C — Spec recording (after Group A and B confirm green)

- [ ] 2.1 Run `speq plan validate add-scalar-connect-back` and fix any delta marker or formatting issues.
- [ ] 2.2 Run `/speq:record add-scalar-connect-back` to merge the spec deltas into the permanent specs library and promote decision-log ADR-040.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A — verify existing code/tests | 1.1, 1.2, 1.3, 1.4 |
| Group B — documentation updates | 1.5, 1.6 |
| Group C — spec merge (sequential after A, B) | 2.1, 2.2 |

Sequential dependencies:
- Group A + Group B → Group C (record only after verification confirms green)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Stale rule | `CLAUDE.md` line 20: "never SCALAR (SCALAR → SIGABRT)" | Empirically disproved; replaced by verified-correct rule in task 1.5 |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Connect-back SCALAR UDF queries the database and returns the result | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_scalar_queries_and_returns` |
| connect-back-scalar returns a value fetched over connect-back from a SCALAR script | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_scalar_queries_and_returns` |
| Connect-back is available identically in scalar and set dispatch | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_scalar_queries_and_returns` (scalar) + `connect_back_udf_queries_and_emits` (set) |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| integration/connect-back (SCALAR) | `make test-e2e` or `cargo test -p it --features integration,db-2026-1` | `[it] scenario connect_back_scalar ok` in output; `test result: ok` |
| examples/test-udfs (connect-back-scalar build) | `cargo build --release -p connect-back-scalar` | Exit 0; `target/release/libconnect_back_scalar.so` present |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt` | No changes |
