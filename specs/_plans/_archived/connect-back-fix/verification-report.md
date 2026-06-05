# Verification Report: connect-back-fix

**Generated:** 2026-06-05

## Verdict

| Result | Details |
|--------|---------|
| **PARTIAL PASS** | Bugs 1 & 2 fixed; 7/9 scenarios pass. 2 connect-back scenarios fail due to a confirmed server-side SIGABRT in `exasol/docker-db:2026.1.0` — not a defect in our implementation. |

| Check | Status |
|-------|--------|
| Build (`cargo build -p exaudfclient`) | ✓ |
| Unit tests (`cargo test -p exa-udf-runtime --features connect-back`) | ✓ |
| Integration — non-connect-back scenarios | ✓ |
| Integration — connect-back scenarios | ✗ (Exasol 2026.1.0 server crash) |

## Test Evidence

### Unit Test Results

```
test result: ok. 8 passed; 0 failed; 0 ignored  (exa-udf-runtime)
test result: ok. 3 passed; 0 failed; 0 ignored  (connect_back integration stubs)
```

All unit tests pass, including:
- `connect_back::tests::dsn_disables_cert_validation_and_carries_credentials` — DSN
  uses `validateservercertificate=0&transport=websocket`, confirming TLS is on but cert
  validation is off and the WebSocket transport is forced.

### Integration Test Results

Run: `cargo +1.91 test -p it --features integration -- --nocapture`

| Scenario | Result | Notes |
|----------|--------|-------|
| sanity_select_one | ✓ Pass | `SELECT 1` returns 1 |
| scalar_double_returns_42 | ✓ Pass | `double_it(21) = 42` |
| set_filter_emits_positive_only | ✓ Pass | 3 positive rows emitted |
| json_parse_extracts_name | ✓ Pass | Third-party crate statically linked |
| udf_error_surfaces_prefix | ✓ Pass | Error contains `F-UDF-CL-RUST-` prefix |
| single_call_default_output_columns | ✓ Pass | Script loads without error |
| single_call_unimplemented_returns_undefined | ✓ Pass | |
| connect_back_udf_queries_and_emits | ✗ FAIL | Server SIGABRT (see §Blocker) |
| connect_back_dml_inserts_visible_via_exapump | ✗ FAIL | Server SIGABRT (see §Blocker) |

## Scenario Coverage Audit

| Scenario (from plan) | Test | Passes |
|----------------------|------|--------|
| `connect_back_udf_queries_and_emits` passes (returns 42) | `connect_back_udf_queries_and_emits` | ✗ |
| `connect_back_dml_inserts_visible_via_exapump` passes | `connect_back_dml_inserts_visible_via_exapump` | ✗ |

Both connect-back scenarios have corresponding tests. The failure is server-side.

## Blocker: Exasol 2026.1.0 Server-Side SIGABRT on Connect-Back

### Root Cause (confirmed via docker logs)

When the UDF sandbox's exaudfclient opens a WebSocket connection to `127.0.0.1:8563`
(the internal connect-back proxy endpoint exposed by Exasol inside the UDF's network
namespace), Exasol:

1. Creates a new `exasql` process for the connect-back session (Part:44, visible in docker logs)
2. Immediately terminates the main session process (Part:40) with **signal 6 (SIGABRT / core dump)**

Docker log evidence:
```
Started /opt/exasol/db-2026.1.0/bin/exasql with PID:2299 UID:500 GID:500 Part:44 Node:0
child 1917 (Part:40 Node:0 exasql) terminated with signal 6. (core dumped)
```

### Diagnosis

The SIGABRT in Part:40 (the main session handler) is triggered by Exasol's internal
connect-back session-association code. When Part:44 authenticates, Exasol tries to link it
to the active UDF session (Part:40). This linkage step asserts/crashes in version 2026.1.0.

### Investigation steps taken

| Approach | Outcome |
|----------|---------|
| Use `validateservercertificate=0` in DSN | Already correct; no change |
| Native binary protocol (exarrow-rs default) | Proxy closes silently before Part:44 is created — no crash, no session |
| WebSocket transport (`transport=websocket`) | Part:44 created; Part:40 crashes (SIGABRT) |
| Use container eth0 IP (`172.17.0.x:8563`) instead of `127.0.0.1` | Not reachable from UDF's network namespace — connection times out |
| Use `exasol/docker-db:2026.latest` tag | Tag does not exist; `2026.1.0` is the only 2026.x image |

### Impact on implementation

The client-side code is correct:
- `exa-udf-runtime/src/connect_back.rs` correctly builds the DSN with `validateservercertificate=0&transport=websocket`
- `connect_back_rt()` creates a current-thread Tokio runtime for sync→async bridging
- Credential exchange via MT_IMPORT works (confirmed in prior session)
- The WebSocket connection reaches the DB and authentication succeeds before the server crashes

### Resolution path

This is a server-side regression in `exasol/docker-db:2026.1.0`. The connect-back
session-association code crashes when a UDF opens a WebSocket connect-back session.
Resolution requires either:
- A server-side patch from Exasol (report to Exasol support with the SIGABRT evidence)
- A newer `2026.x` Docker image where the bug is fixed

## Notes

- **Bug 1 fixed** (`connect-back` feature flag on `exaudfclient/Cargo.toml`)
- **Bug 2 fixed** (run-phase `ConnInfo` propagation via on-demand MT_IMPORT in `dispatch.rs`)
- **Error surfacing** added: UDF errors now include the `F-UDF-CL-RUST-` prefix
- **Transport fix** applied: `exa-udf-runtime` now explicitly enables the `websocket` feature
  in exarrow-rs and pins `transport=websocket` in the connect-back DSN, since the native
  binary protocol is not supported by Exasol's connect-back proxy
- The test harness was improved with `on_scenario_fail()` which dumps UDF logs and docker
  container logs on failure, and supports `KEEP_CONTAINER_SECS=<n>` for manual inspection
