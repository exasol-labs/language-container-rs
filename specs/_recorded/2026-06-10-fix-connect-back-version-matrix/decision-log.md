# Decision Log: fix-connect-back-version-matrix

Date: 2026-06-09

## Interview

**Q:** ADR-015 SIGABRT — is it a real server bug?
**A:** No. SIGABRT is not a bug; it was the wrong way of talking to the cluster. We can open a connection to the cluster based on its IP like any other client would. Remove the SIGABRT content from the project — it confuses everyone. connect_back is doable via exarrow-rs and regular TCP connections, against the regular IP of the cluster (or the IP of a remote machine). The ITs and E2E tests must prove that.

**Q:** Fix for the cluster_ip IPC issue?
**A:** IPC — I would like to move that out and use TCP, so `cluster_ip()` always receives a `tcp://` endpoint and can parse the node IP.

**Q:** Version matrix mechanism?
**A:** Cargo feature flags per version (`db-2025-1`, `db-2025-2`, `db-2026-1`).

**Q:** CI system?
**A:** GitHub Actions (already exists at `.github/workflows/ci.yml`), with `2026.1` as the default.

## Design Decisions

### [1] Connect-back targets the node's own SQL endpoint over TCP

- **Decision:** `CB_SELF` is created `TO '<container-eth0-ip>:8563'` (from `container_inner_ip()`), and the connect-back `exarrow-rs` session connects over plain TCP as a regular external client. The query and DML scenarios become hard assertions on every version.
- **Alternatives:** Docker host gateway + host-mapped port (the current NAT path that crashed the parent session); container loopback / internal-proxy framing.
- **Rationale:** The user confirmed the node's own routable SQL IP is the supported client endpoint. The earlier crash was caused by the gateway/NAT route, not a database defect.
- **Promotes to ADR:** yes

### [2] The UDF↔DB ZMQ transport cannot be forced to TCP via SCRIPT_LANGUAGES (premise correction)

- **Decision:** Do NOT attempt to change `localzmq` to a TCP variant in the `SCRIPT_LANGUAGES` string. Instead, `cluster_ip()` asserts a hard IPv4 result only where the database launches the container with a `tcp://` endpoint, and asserts the deterministic `ipc://`-parse-error on single-node `exasol/docker-db`. Severity is selected at runtime by `EXASOL_DB_SERIES`.
- **Alternatives:** Swap `localzmq` for a TCP transport prefix (the literal interview request); unconditionally skip `cluster_ip()` on Docker.
- **Rationale:** Upstream `exaudflib_main.cc` shows `argv[1]`'s transport scheme is chosen by the database at launch, not by `SCRIPT_LANGUAGES`. A `tcp:` `argv[1]` selects *remote-client* mode (the client binds, the DB connects to it) — a deployment model `exasol/docker-db` single-node does not use. For a locally-launched (`localzmq`) container the DB always passes `ipc://`. There is no SCRIPT_LANGUAGES flag that flips this. Asserting both branches tests the real behaviour instead of skipping.
- **Superseded for `cluster_ip` by [5]:** the two-branch / severity-flag assertion for `cluster_ip()` is replaced by reading the node IP from the network interface, yielding one hard IPv4 assertion. The premise correction (cannot force TCP ZMQ via `SCRIPT_LANGUAGES`) still stands; only the assertion strategy changed.
- **Promotes to ADR:** yes

### [3] Cargo features declare supported versions; runtime env var selects the active one

- **Decision:** Add `db-2025-1`, `db-2025-2`, `db-2026-1` features (`default = ["db-2026-1"]`) as compile-time capability declarations. Actual per-version branching (image tag + `cluster_ip` assertion severity) happens at runtime via `EXASOL_DB_SERIES`, falling back to the compiled default when unset.
- **Alternatives:** Compile one `it-runner` per version (matrix in `build-artifacts`); use `cfg`-gated test bodies per feature.
- **Rationale:** `build-artifacts` compiles `it-runner` once and every matrix job reuses that single binary. A reused binary cannot carry per-matrix `cfg`. Per-version binaries triple build time and cache size for no behavioural gain. The hybrid honours "Cargo feature per version" while keeping one artifact.
- **Promotes to ADR:** yes

### [4] Remove all SIGABRT / ADR-015 narrative from code and specs

- **Decision:** Delete `is_known_sigabrt_failure()`, `is_known_ipc_transport_failure()`, the SIGABRT match arms, the ADR-015 ordering comment, and the "reaches a routable endpoint without crashing" scenario; strip SIGABRT/ADR-015 mentions from runtime comments and integration specs.
- **Alternatives:** Keep the helpers behind a flag for historical reference.
- **Rationale:** The user explicitly asked to remove the SIGABRT content because it confuses everyone; keeping it re-introduces the misleading narrative. ADR-013/ADR-015 in the permanent `specs/decision-log.md` are superseded and will be cleaned up at `speq record` time.
- **Promotes to ADR:** no

### [5] `cluster_ip()` reimplemented via network interface rather than ZMQ endpoint parsing

- **Decision:** `cluster_ip()` (in `crates/exa-udf-runtime/src/rowset.rs`) reads the local node's primary IPv4 from the network interface — the first non-loopback IPv4 of the UDF process (e.g. container `eth0`), via `libc::getifaddrs` — instead of parsing the IP out of the ZMQ endpoint string. `parse_cluster_ip()` in `crates/exa-udf-runtime/src/artifact.rs` is removed as dead code. The `connect_back_cluster_ip_emits_node_ip` scenario becomes a hard IPv4 assertion on every series, with no `cluster_ip_endpoint_is_tcp()` severity flag and no `ipc://`-error branch.
- **Alternatives:** Parse the IP from the ZMQ endpoint string (the original implementation, which fails on single-node Docker because the DB passes `ipc://` with no IP); force a TCP ZMQ transport via `SCRIPT_LANGUAGES` (infeasible — the DB chooses `argv[1]`); assert two branches with a runtime severity flag (superseded by this decision); unconditionally skip on Docker.
- **Rationale:** The UDF runs as a normal Linux process inside the Exasol container and has full access to interface-enumeration syscalls. `libc` is already a workspace dependency (`libc = "0.2"`), so `getifaddrs` is available with no new dependency. Reading the interface returns a valid IPv4 on single-node Docker and multi-node TCP clusters alike, so the scenario collapses to one hard assertion. This removes the topology-dependent severity branch (decision [2]) for `cluster_ip` specifically; the `EXASOL_DB_SERIES` flag remains available for other future version-specific behaviour but no longer gates `cluster_ip`.
- **Promotes to ADR:** yes

### [6] CB_SELF address is deployment-mode-aware

- **Decision:** The `CB_SELF` address is not hard-coded; the harness selects it per deployment mode via a new `Harness::connect_back_sql_address()` method. In testcontainers mode it returns `container_inner_ip():8563` — the container's own `eth0` address with the internal SQL port, a direct TCP path that bypasses Docker NAT (this is the actual crash fix). In external mode (`EXASOL_HOST` set), it returns the harness's already-known `host:db_port` — the cluster's routable SQL endpoint, reachable from within the node. No new env var is introduced.
- **Alternatives:** Hard-code `container_inner_ip():8563` for all modes (the prior plan text); this breaks on real non-Docker clusters because `container_inner_ip()` works by `docker exec <container> ip addr show eth0`, and a real cluster has no container to exec into. Reuse `host:db_port` for all modes; this breaks in testcontainers mode because there `host:db_port` is the NAT-mapped ephemeral host port (the original crashing NAT path), not the internal `8563`.
- **Rationale:** The two modes need different addresses for the same goal (a direct TCP path to the node's SQL endpoint). `container_inner_ip()` is Docker-only by construction; `host:db_port` is correct only when it already points at the cluster SQL endpoint (external mode), not the NAT-mapped port (testcontainers mode). Branching on `self._container.is_some()` reuses state the `Harness` already carries, requires no new env var, and makes the plan correct on both local Docker and real clusters.
- **Promotes to ADR:** yes

## Review Findings

<!-- Populated by speq-implement after code review. -->
