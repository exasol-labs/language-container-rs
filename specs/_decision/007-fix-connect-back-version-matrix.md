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

An interview request asked to move `cluster_ip()` to always receive a `tcp://` ZMQ endpoint by changing the `SCRIPT_LANGUAGES` string. Investigation of `exaudflib_main.cc` showed this is not possible: `argv[1]`'s transport scheme is chosen by the database at launch, not by `SCRIPT_LANGUAGES`. On single-node `exasol/docker-db`, the database always passes `ipc://` for a locally-launched (`localzmq`) container.

### Decision

Do not attempt to change the `localzmq` transport prefix in `SCRIPT_LANGUAGES`. The ZMQ endpoint transport is a database-side concern. For `cluster_ip()`, the solution (reading the network interface instead of parsing the endpoint) is captured in ADR-025. The premise correction — that the transport cannot be forced — stands independently.

### Options Considered

| Option | Verdict |
|--------|---------|
| Accept that ZMQ transport is DB-controlled; address `cluster_ip()` separately | ✓ Chosen — correct description of the constraint; avoids impossible workarounds |
| Swap `localzmq` for a TCP transport prefix in `SCRIPT_LANGUAGES` | ✗ Rejected — `argv[1]`'s scheme is chosen by the DB at launch; `SCRIPT_LANGUAGES` has no flag to flip this |
| Use a `tcp:` `argv[1]` to select remote-client mode | ✗ Rejected — remote-client mode is a deployment model `exasol/docker-db` single-node does not use |

### Consequences

The ZMQ socket transport (IPC on single-node Docker, TCP on multi-node clusters) remains DB-controlled and opaque to the SLC. No code or configuration change can force a TCP ZMQ endpoint on single-node Docker. The `cluster_ip()` fix (ADR-025) does not depend on the ZMQ transport at all.

## ADR: Cargo features declare supported versions; runtime env var selects the active one

**ID:** cargo-features-declare-versions-env-var-selects
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The CI matrix runs three Exasol versions (`2025.1`, `2025.2`, `2026.1`). The `build-artifacts` job compiles `it-runner` once and every matrix job reuses that single binary. Cargo features are compile-time, so a reused binary cannot have a different feature set per matrix entry.

### Decision

Add `db-2025-1`, `db-2025-2`, `db-2026-1` features to `crates/it/Cargo.toml` with `default = ["db-2026-1"]`. These features are capability declarations only — no `cfg`-gated test bodies. Actual per-version branching (image tag selection) happens at runtime via `EXASOL_DB_SERIES`, falling back to the compiled default when unset. Unknown values are rejected with a clear error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Compile-time features as declarations; runtime `EXASOL_DB_SERIES` for selection | ✓ Chosen — single artifact; honours "Cargo feature per version" intent; runtime env is flexible per matrix entry |
| Compile one `it-runner` per version (matrix in `build-artifacts`) | ✗ Rejected — triples build time and cache size for no behavioural gain |
| Use `cfg`-gated test bodies per feature | ✗ Rejected — binary compiled once cannot carry per-matrix `cfg` |

### Consequences

`build-artifacts` compiles `it-runner` once with `--features integration,db-2026-1`. Every matrix job sets `EXASOL_DB_SERIES` to select version behaviour at runtime. Local `cargo test` with no env var runs the `2026-1` series (the default). Unrecognised values fail fast.

## ADR: cluster_ip() reads the node IP from the network interface instead of parsing the ZMQ endpoint

**ID:** cluster-ip-reads-network-interface
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The original `cluster_ip()` parsed the node IP out of the ZMQ endpoint string (`argv[1]`). On single-node `exasol/docker-db` the database passes `ipc://` with no node IP to parse, so `cluster_ip()` returned an error there. ADR-023 establishes that the transport cannot be forced to TCP. A different implementation strategy was needed.

### Decision

`cluster_ip()` (in `crates/exa-udf-runtime/src/rowset.rs`) reads the local node's primary IPv4 from the network interface — the first non-loopback IPv4 of the UDF process (e.g. container `eth0`) — via `libc::getifaddrs`, instead of parsing the ZMQ endpoint string. `parse_cluster_ip()` in `crates/exa-udf-runtime/src/artifact.rs` is removed as dead code. The `connect_back_cluster_ip_emits_node_ip` scenario becomes a hard IPv4 assertion on every series.

### Options Considered

| Option | Verdict |
|--------|---------|
| Read primary IPv4 from network interface via `libc::getifaddrs` | ✓ Chosen — works identically on single-node Docker and multi-node TCP clusters; `libc` is already a workspace dependency; collapses to one hard assertion |
| Parse IP from ZMQ endpoint string | ✗ Rejected — fails on single-node Docker because the DB passes `ipc://` with no IP |
| Force TCP ZMQ transport via `SCRIPT_LANGUAGES` | ✗ Rejected — infeasible; see ADR-023 |
| Assert two branches with runtime severity flag (`EXASOL_DB_SERIES`) | ✗ Rejected — superseded by this approach; reading the interface eliminates the topology-dependent branch |
| Unconditionally skip `cluster_ip()` on Docker | ✗ Rejected — removes test coverage on the most common development environment |

### Consequences

`cluster_ip()` returns a valid IPv4 on both single-node Docker and multi-node TCP deployments. `parse_cluster_ip()` is dead code and is removed. The `connect_back_cluster_ip_emits_node_ip` integration scenario has no severity branch and no unconditional skip — it is a hard assertion on every version in the matrix. The `EXASOL_DB_SERIES` flag remains available for other future version-specific behaviour but no longer gates `cluster_ip`.

## ADR: CB_SELF address is deployment-mode-aware via Harness::connect_back_sql_address()

**ID:** cb-self-address-deployment-mode-aware
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The `CB_SELF` address must be a direct TCP path to the node's SQL endpoint reachable from the UDF sandbox. In testcontainers mode the harness `host:db_port` is the NAT-mapped ephemeral host port (the original crashing path), so the container's `eth0` address must be used instead. In external mode (a real cluster, `EXASOL_HOST` set) there is no container to `docker exec` into, so `container_inner_ip()` would fail and `host:db_port` is the correct address.

### Decision

Add `Harness::connect_back_sql_address()` to `crates/it/src/lib.rs`. In testcontainers mode (`self._container.is_some()`) it returns `format!("{}:8563", self.container_inner_ip().await?)`. In external mode (`self._container.is_none()`) it returns `format!("{}:{}", self.host, self.db_port)`. No new env var is introduced. `container_connect_back_address()` is removed as dead code.

### Options Considered

| Option | Verdict |
|--------|---------|
| Mode-aware `connect_back_sql_address()` branching on `self._container.is_some()` | ✓ Chosen — reuses state the `Harness` already carries; correct for both local Docker and real clusters; no new env var |
| Hard-code `container_inner_ip():8563` for all modes | ✗ Rejected — `container_inner_ip()` requires `docker exec`; fails on real non-Docker clusters |
| Reuse `host:db_port` for all modes | ✗ Rejected — in testcontainers mode `host:db_port` is the NAT-mapped ephemeral port (the original crashing path) |

### Consequences

`CB_SELF` is always a direct TCP path to the node's SQL endpoint regardless of deployment mode. `container_connect_back_address()` is removed. The mode distinction is transparent to test scenarios — they call `connect_back_sql_address()` uniformly.
