# Decisions: fix-it-matrix-connect-back-address

## ADR: External-mode connect-back uses the container eth0 IP; one root cause, not two

**ID:** external-mode-connect-back-eth0-ip-one-root-cause
**Plan:** `fix-it-matrix-connect-back-address`
**Status:** Accepted

### Context

A connect-back address of `localhost:8563` resolves to `127.0.0.1`, Exasol's internal CoreDB proxy. The proxy links the connect-back session to the invoking SQL worker and triggers a VM SIGABRT. Integration scenarios share one `Connection`, so the crash poisons the session and later scenarios fail on the dead VM. The integration suite's external mode always targets the local Docker container `exasol-db`, so `container_inner_ip()` works via `docker exec exasol-db`.

### Decision

`Harness::connect_back_sql_address()` resolves the container `eth0` IP via `container_inner_ip()` and returns `<container-eth0-ip>:8563` in both testcontainers and external mode. It never returns a loopback address. If IP resolution fails, it errors instead of falling back to `localhost`. The Python3 connect-back diagnostic runs on a dedicated throwaway `harness.connect()` connection, so a VM crash cannot poison the shared connection.

### Options Considered

| Option | Verdict |
|--------|---------|
| External mode uses `container_inner_ip()` | ✓ Chosen, loopback is the failing address |
| `host:db_port` with an `EXASOL_CB_ADDRESS` override | ✗ `host:db_port` is the crashing `localhost:8563` |
| Diagnostic on a throwaway connection | ✓ Chosen, isolates the crash |
| Reconnect the main `conn` after the diagnostic | ✗ Does not prevent poisoning mid-setup |
| Rewrite the rowset scalar path | ✗ Scalar failures are collateral of the poisoned session |

### Consequences

A remote non-Docker external mode is out of scope. An `EXASOL_CB_ADDRESS` override would cover it. The rowset codec is unchanged.
