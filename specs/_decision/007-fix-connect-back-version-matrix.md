# Decisions: fix-connect-back-version-matrix

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
