# Decisions: fix-connect-back-external-client

## ADR: Connect-back is always a new external-client session and a new transaction

**ID:** connect-back-new-external-client-session-transaction
**Plan:** `fix-connect-back-external-client`
**Status:** Accepted

### Context

Exasol core cannot share the invoking query's transaction with a container UDF. The internal-proxy path at loopback/eth0 `:8563` causes a SIGABRT.

### Decision

The runtime opens connect-back as an external-client login to the `address`/`user`/`password` returned by `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`), which creates a new session and a new transaction. This matches PyExasol's `exa.get_connection(NAME)` followed by an independent connect.

### Options Considered

| Option | Verdict |
|--------|---------|
| New external-client session and transaction | ✓ Chosen, matches reference SLCs (Python/Java/strata-rs) |
| Share the invoking session or transaction | ✗ Core cannot share it, internal proxy causes the SIGABRT |

### Consequences

The `CB_SELF` named connection must be created `TO '<routable-endpoint>:8563'`, reachable from the UDF sandbox network namespace. Connect-back queries do not see the caller's uncommitted state. Operators configure the endpoint, and the UDF artifact stays generic via `%connection <NAME>`.

