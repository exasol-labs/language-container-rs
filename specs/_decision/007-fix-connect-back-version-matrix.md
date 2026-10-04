# Decisions: fix-connect-back-version-matrix

## ADR: Connect-back targets the node's own SQL endpoint over TCP

**ID:** connect-back-targets-node-sql-endpoint-tcp
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted
**Supersedes:** docker-host-gateway-does-not-resolve-sigabrt

### Context

The earlier connect-back implementation created `CB_SELF TO '<docker-host-gateway>:<mapped-port>'` and routed the connect-back `exarrow-rs` session through Docker NAT. This path caused the parent SQL session to terminate with signal 6. The root cause was the routing choice, not a database defect. Supersedes the framing of ADR-015.

### Decision

`CB_SELF` is created `TO '<connect_back_sql_address()>'`. The harness selects the address per deployment mode: in testcontainers mode it returns `<container-eth0-ip>:8563` (the container's own `eth0` address, bypassing NAT); in external mode (`EXASOL_HOST` set) it returns `<host>:<db_port>` (the cluster's routable SQL endpoint the harness already carries). The connect-back `exarrow-rs` session connects over plain TCP as a regular external client. The query and DML scenarios become hard assertions on every version.

### Options Considered

| Option | Verdict |
|--------|---------|
| Deployment-mode-aware address via `Harness::connect_back_sql_address()` | ✓ Chosen — direct TCP to the node's own SQL endpoint is the supported client path; mode distinction is essential because `container_inner_ip()` is Docker-only |
| Docker host gateway + host-mapped port | ✗ Rejected — the NAT path that caused the original SIGABRT |
| Hard-code `container_inner_ip():8563` for all modes | ✗ Rejected — `container_inner_ip()` requires `docker exec`; fails on real non-Docker clusters |
| Container loopback / internal-proxy framing | ✗ Rejected — caused the original SIGABRT in ADR-015 |

### Consequences

Connect-back query and DML scenarios pass as hard assertions on all three versions in the matrix (`2025.1`, `2025.2`, `2026.1`). `container_connect_back_address()` and the Docker-gateway address helper are removed as dead code. ADR-015 is superseded.

## ADR: The UDF↔DB ZMQ transport cannot be forced to TCP via SCRIPT_LANGUAGES

**ID:** zmq-transport-cannot-be-forced-tcp
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The database chooses the transport scheme of `argv[1]` (see `exaudflib_main.cc`) at launch, not `SCRIPT_LANGUAGES`. On single-node `exasol/docker-db`, the database passes `ipc://` for a locally-launched (`localzmq`) container.

### Decision

The `localzmq` transport prefix in `SCRIPT_LANGUAGES` is left unchanged. The ZMQ endpoint transport is a database-side concern.

### Options Considered

| Option | Verdict |
|--------|---------|
| Treat ZMQ transport as DB-controlled | ✓ Chosen |
| Swap `localzmq` for a TCP prefix in `SCRIPT_LANGUAGES` | ✗ `SCRIPT_LANGUAGES` cannot change the scheme |
| `tcp:` `argv[1]` to select remote-client mode | ✗ Single-node `exasol/docker-db` does not use remote-client mode |

### Consequences

The ZMQ transport (IPC on single-node Docker, TCP on multi-node clusters) is opaque to the SLC.

## ADR: cluster_ip() reads the node IP from the network interface instead of parsing the ZMQ endpoint

**ID:** cluster-ip-reads-network-interface
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

On single-node `exasol/docker-db` the ZMQ endpoint is `ipc://` and contains no node IP, so parsing it fails. The transport cannot be forced to TCP.

### Decision

`cluster_ip()` (in `crates/exa-udf-runtime/src/rowset.rs`) returns the first non-loopback IPv4 of the UDF process (e.g. container `eth0`), read via `libc::getifaddrs`. `parse_cluster_ip()` in `crates/exa-udf-runtime/src/artifact.rs` is removed. The `connect_back_cluster_ip_emits_node_ip` scenario is a hard IPv4 assertion on every series.

### Options Considered

| Option | Verdict |
|--------|---------|
| Read primary IPv4 via `libc::getifaddrs` | ✓ Chosen, same on Docker and multi-node, `libc` already a dependency |
| Parse the ZMQ endpoint string | ✗ No IP in `ipc://` |
| Force TCP ZMQ via `SCRIPT_LANGUAGES` | ✗ Not possible |
| Two assertion branches with an `EXASOL_DB_SERIES` severity flag | ✗ Topology-dependent branch |
| Skip `cluster_ip()` on Docker | ✗ Loses coverage on the common development environment |

### Consequences

`cluster_ip()` returns a valid IPv4 on single-node Docker and multi-node TCP. `EXASOL_DB_SERIES` no longer gates `cluster_ip`.
