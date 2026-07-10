# Decisions: fix-connect-back-external-client

## ADR: Connect-back is always a new external-client session and a new transaction

**ID:** connect-back-new-external-client-session-transaction
**Plan:** `fix-connect-back-external-client`
**Status:** Accepted

### Context

Connect-back lets UDFs query the database from inside `run()`. A question arose about whether the runtime should attempt to share the invoking query's session or transaction, or open an independent external-client connection.

### Decision

The runtime opens connect-back as an ordinary external-client login to the `address`/`user`/`password` returned by `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`), establishing a new session and a new transaction. This is the same pattern as PyExasol's `exa.get_connection(NAME)` followed by an independent connect.

### Options Considered

| Option | Verdict |
|--------|---------|
| New external-client session, new transaction | ✓ Chosen — matches reference SLCs (Python/Java/strata-rs); core cannot share a UDF transaction anyway |
| Share/join the invoking query's session or transaction | ✗ Rejected — Exasol core cannot share the invoking query's transaction with a container UDF; the internal-proxy path at loopback/eth0 `:8563` caused the original SIGABRT |

### Consequences

The `CB_SELF` named connection must be created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox network namespace. Connect-back queries run in a separate transaction and do not see the caller's uncommitted state. Operators configure the endpoint; the UDF artifact stays generic via `%connection <NAME>`.

## ADR: Docker-host-gateway address does not resolve the 2026.latest SIGABRT

**ID:** docker-host-gateway-does-not-resolve-sigabrt
**Plan:** `fix-connect-back-external-client`
**Status:** Accepted

### Context

Commit `7de7357` changed the connect-back address to the Docker host gateway (instead of the container's loopback/eth0), hypothesising this would let the connect-back act as an external client and avoid the server-side SIGABRT. This plan ran a fresh integration suite on `2026-06-06` to verify the hypothesis.

### Decision

Record empirically that the SIGABRT persists on `exasol/docker-db:2026.latest` (image id `b81d80f63d10`, identical to `2026.1.0`) even with the Docker gateway external-client address. The crash is server-side, signal 6, and triggered by the core spawning a connect-back session for any container UDF — independent of address or transport. The SLC implementation is correct; the blocker is an upstream core defect.

Evidence from the `2026-06-06` run:
- 6 / 8 scenarios PASS (scalar, set, json, udf-error, both single-call).
- Both connect-back scenarios FAIL with `peer closed connection without sending TLS close_notify` on the outer session.
- Container log: `child <pid> (Part:40 Node:0 exasql) terminated with signal 6. (core dumped)` immediately after `Part:44` (connect-back session process) is spawned.

### Options Considered

| Option | Verdict |
|--------|---------|
| Record crash as unresolved upstream blocker; keep scenarios as known-failing gates | ✓ Chosen — honest evidence; scenarios auto-turn-green on a patched image |
| Assume the gateway fix resolved it (prior hypothesis) | ✗ Rejected — direct re-verification contradicts the hypothesis |
| Delete connect-back scenarios | ✗ Rejected — they form a regression net for when a patched image ships |

### Consequences

Connect-back integration scenarios remain known-failing on `2026.latest`. No workaround exists within the SLC. The test suite dumps SIGABRT diagnostics on failure. Once Exasol ships a patched image, the scenarios should pass without any SLC code changes.
