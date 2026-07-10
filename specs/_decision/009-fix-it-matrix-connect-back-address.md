# Decisions: fix-it-matrix-connect-back-address

## ADR: External-mode connect-back uses the container eth0 IP; one root cause, not two

**ID:** external-mode-connect-back-eth0-ip-one-root-cause
**Plan:** `fix-it-matrix-connect-back-address`
**Status:** Accepted

### Context

The new exarrow-style CI pipeline (PR #1, merged) failed the `integration` job on all three DB versions (`2025.1.11`, `2025.2.1`, `2026.1.0`), blocking the `release` job. The failing CI run (27425472641) showed two integration scenarios failing:

```
scenario python3_connect_back FAILED: ... VM crashed (Session: 1867805944649744384)
Error: query: SELECT TO_CHAR(double_it(21)) ... VM crashed (Session: 1867805944649744384)
```

Both failures carried the **same DB Session ID**, proving they hit one shared, poisoned connection rather than two independent bugs. The root cause was `connect_back_sql_address()` returning `localhost:8563` in external mode (`EXASOL_HOST=localhost`), which resolves to `127.0.0.1` — Exasol's internal CoreDB proxy. The proxy links the connect-back session to the invoking SQL worker (Part:40), triggering a VM SIGABRT. Because all scenarios ran sequentially on one shared `Connection`, the crash from scenario 2 (`python3_connect_back`) poisoned the session and scenario 3 (`double_it` / `scalar_double`) then failed on the dead VM.

A prior plan (`fix-connect-back-version-matrix`, 2026-06-10) had assumed that external mode meant a remote cluster with no Docker container to exec into, so it returned `host:db_port`. In fact, the IT suite's external mode always targets a local Docker container named `exasol-db` in both CI and local-repro configurations — `container_inner_ip()` was already usable via `docker exec exasol-db`, making `localhost:8563` the uniquely wrong address.

The rowset row-major codec (`crates/exa-udf-runtime/src/rowset.rs`) was considered but deliberately left untouched: the evidence (shared Session ID, scenario-2-crashes-first) demonstrated the scalar path was collateral damage, not the root cause.

### Decision

`Harness::connect_back_sql_address()` always resolves the container `eth0` IP via `container_inner_ip()` and returns `<container-eth0-ip>:8563` in **both** testcontainers and external mode. It must never return a loopback address; if IP resolution fails it errors loudly rather than falling back to `localhost`. The Python3 connect-back diagnostic runs on a dedicated throwaway `harness.connect()` connection so that any VM crash from that diagnostic cannot poison the shared connection used by the asserted scenarios.

### Options Considered

| Option | Verdict |
|--------|---------|
| External mode uses `container_inner_ip()` (same as testcontainers) | ✓ Chosen — container is always `exasol-db` and exec-able in CI/local; loopback is the only failing address; simplest correct fix |
| Keep `host:db_port` in external mode; add `EXASOL_CB_ADDRESS` env override | ✗ Rejected — `host:db_port` is `localhost:8563` in CI, which is exactly the crashing address; env override deferred to a future genuine remote-cluster use case |
| Diagnostic on throwaway connection | ✓ Chosen — isolation is cleaner and removes the cascade that caused the misdiagnosis |
| Reconnect main `conn` after the diagnostic | ✗ Rejected — does not prevent poisoning if the crash happens mid-setup; throwaway isolation is structurally safer |
| Audit/rewrite the rowset scalar path | ✗ Rejected — evidence (shared Session ID, scenario order) shows scalar path is collateral, not cause; `double_it` passes on a healthy session |

### Consequences

`connect_back_sql_address()` is now mode-independent: both testcontainers and external mode use `<container-eth0-ip>:8563`. A genuine remote-cluster (non-Docker) external mode is out of scope; an explicit `EXASOL_CB_ADDRESS` environment-variable override would cover that future use case. The rowset codec was not changed. All 12 integration scenarios pass locally in external mode against `exasol/docker-db:2026.1.0`; the fix is version-independent and expected to green the full CI matrix.
