# Plan: fix-connect-back-version-matrix

## Summary

Make the three connect-back integration scenarios pass as hard assertions by pointing the connect-back `exarrow-rs` session at the cluster node's own SQL endpoint over TCP (the regular Exasol client path), remove all SIGABRT / ADR-015 framing from code and specs, and extend the integration harness with a per-version mechanism (Cargo features declaring supported versions + a runtime `EXASOL_DB_SERIES` switch) so the same compiled `it-runner` binary drives the `2025.1`, `2025.2`, and `2026.1` matrix locally and in GitHub Actions.

## Design

### Context

Two distinct problems are conflated in the current state, and one stated premise is technically infeasible — the design must separate them.

- **Connect-back round-trip** previously created `CB_SELF` pointing at the Docker host gateway plus the host-mapped DB port (NAT path). The parent session was killed by signal 6 the moment the core spawned the connect-back process. The user states this is NOT a database bug — it was the wrong way of reaching the cluster. The supported path is to connect over plain TCP to the node's own routable SQL endpoint (`<container-eth0-ip>:8563`), exactly as any external client would. `container_inner_ip()` already returns that eth0 address.
- **`cluster_ip()` on single-node Docker** returned an error because the previous implementation parsed the node IP out of the ZMQ endpoint string (`argv[1]`), and the database launches the locally-launched (`localzmq`) container with an `ipc://` socket whose path contains no node IP to parse. The transport scheme of `argv[1]` is chosen by the database at launch, not by `SCRIPT_LANGUAGES`, so it cannot be forced to TCP. The fix is to stop depending on the endpoint string altogether: `cluster_ip()` now reads the local node's primary IPv4 from the network interface (the first non-loopback IPv4 of the UDF process, e.g. container `eth0`). The UDF runs as a normal Linux process inside the Exasol container with full access to interface-enumeration syscalls (`libc::getifaddrs`, already a workspace dependency), so it returns a valid IPv4 on single-node Docker and multi-node TCP clusters alike.
- **Version matrix.** The `it` crate is compiled once in `build-artifacts`; the `it-runner` binary is uploaded and reused by every matrix job. Cargo features are compile-time, so a single reused binary cannot have a different feature set per matrix entry.

- **Goals**
  - Connect-back query and DML scenarios pass as hard assertions on all three versions, connecting over TCP to `<container-eth0-ip>:8563`.
  - All SIGABRT / ADR-015 / known-failing language removed from code and specs.
  - One compiled `it-runner` drives every matrix version; both local `cargo test` and GitHub Actions exercise `2025.1`, `2025.2`, `2026.1` with `2026.1` as default.
  - `cluster_ip()` returns a valid IPv4 (from the local network interface) as a hard assertion on every version, single-node Docker and multi-node alike.
- **Non-Goals**
  - Forcing a TCP ZMQ transport for the UDF↔DB control channel on single-node Docker (infeasible, and no longer needed now that `cluster_ip()` reads the interface; see Context).
  - Auto-discovering the connect-back address from the endpoint (the `%connection CB_SELF` directive stays the supported path).
  - Multi-node cluster provisioning in CI.

### Decision

#### Architecture

```
build-artifacts (compile ONCE)
  cargo test --no-run -p it --features integration,db-2026-1  ──▶ it-runner (binary)
                                                                       │ uploaded
                                                                       ▼
integration matrix job  (download it-runner, reuse for each version)
  ┌─ EXASOL_DB_SERIES=2025-1  EXASOL_VERSION=2025.1.x ─┐
  ├─ EXASOL_DB_SERIES=2025-2  EXASOL_VERSION=2025.2.x ─┤──▶ ./it-runner   (runtime branch)
  └─ EXASOL_DB_SERIES=2026-1  EXASOL_VERSION=2026.1.0 ─┘
                                              │
                                              ▼
   db_series()  ── reads EXASOL_DB_SERIES (fallback: compiled default feature)
        │
        └─ image tag   (EXASOL_VERSION wins; else series→tag map)

cluster_ip()  ── reads local node primary IPv4 from network interface (libc::getifaddrs)
        └─▶ first non-loopback IPv4 (e.g. eth0)  → hard IPv4 assertion on every series

connect-back (all versions, hard assertion)
  Harness::connect_back_sql_address()  ── deployment-mode-aware CB_SELF address
    ├─ testcontainers mode → '<container_inner_ip()>:8563'  (container eth0, no NAT)
    └─ external mode (EXASOL_HOST set) → '<self.host>:<self.db_port>'  (cluster SQL endpoint)
                                              │
                                              ▼
  CB_SELF TO '<connect_back_sql_address()>'  ──TCP──▶ node SQL endpoint  (exarrow-rs)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Compile-time feature = capability *declaration*; runtime env var = *selection* | `crates/it` features + `db_series()` | Reconciles "Cargo feature per version" with the compile-once/reuse-binary CI constraint |
| Default feature picks the default series | `it/Cargo.toml` `default = ["db-2026-1"]` | Local `cargo test` with no env var runs the 2026.1 series, matching the user's "2026.1 default" |
| Deployment-mode-aware connect-back address | `Harness::connect_back_sql_address()` feeds `CB_SELF` | The supported external-client path; in testcontainers mode it uses `container_inner_ip():8563` (container eth0, bypassing NAT — the actual crash fix); in external mode it uses the harness's known `host:db_port` SQL endpoint, which is the cluster's routable IP reachable from within the node. Avoids the gateway/NAT route that crashed the parent session and works on real non-Docker clusters |
| Single shared container, sequential scenarios | `db_roundtrip.rs` | Container startup is 2-3 min; connect-back no longer crashes the session, so ordering constraints relax |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Runtime `EXASOL_DB_SERIES` drives version branching; Cargo features only declare supported versions + default | (a) Compile one `it-runner` per version (matrix in `build-artifacts`); (b) features do real `cfg` branching | A single reused binary cannot carry per-matrix `cfg`. Per-version binaries triple build time and cache size for no behavioural gain. Runtime env keeps one artifact while honouring "feature per version". |
| `cluster_ip()` reads the local node's primary IPv4 from the network interface (`libc::getifaddrs`, first non-loopback IPv4) and asserts a hard IPv4 result on every series | Parse the IP from the ZMQ endpoint string (current); force TCP ZMQ via `SCRIPT_LANGUAGES`; unconditional skip | Endpoint parsing fails on single-node Docker because the DB passes `ipc://` (no IP). Forcing TCP ZMQ is infeasible (DB chooses `argv[1]`). Reading the interface works identically on single-node Docker and multi-node clusters, so the scenario becomes one hard assertion with no severity branch — `parse_cluster_ip` becomes dead code. |
| `CB_SELF` address is selected by deployment mode via `Harness::connect_back_sql_address()`: `container_inner_ip():8563` in testcontainers mode, `host:db_port` in external mode | A single hard-coded `container_inner_ip():8563` (breaks on non-Docker clusters where there is no container to `docker exec`); Docker host gateway + mapped port (current; the NAT path that crashed the parent session); container loopback | Per the user, the node's own routable SQL IP is the supported client endpoint; the gateway/NAT path was the cause of the crash, not a DB bug. `container_inner_ip()` execs `ip addr` inside the container and so only works in Docker modes — on a real cluster there is no container, so external mode must use the harness's already-known `host:db_port`. In testcontainers mode `host:db_port` is the NAT-mapped ephemeral host port (the broken path), so only `container_inner_ip():8563` bypasses NAT there. The mode distinction is essential and requires no new env var. |
| Delete `container_connect_back_address()` and the SIGABRT/IPC soft-path helpers | Keep them behind a flag | Dead once the address strategy and assertions change; keeping them re-introduces the confusing SIGABRT narrative the user wants removed. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| integration/connect-back | CHANGED | `specs/_plans/fix-connect-back-version-matrix/integration/connect-back/spec.md` |
| integration/db-roundtrip | CHANGED | `specs/_plans/fix-connect-back-version-matrix/integration/db-roundtrip/spec.md` |

## Dependencies

- `exasol/docker-db` images for tags `2025.1.11`, `2025.2.1`, `2026.1.0`.
- `exapump` (DML visibility check) and a privileged Docker daemon — already used.

## Migration

| Current | New |
|---------|-----|
| `EXASOL_VERSION` env var only (image tag) | `EXASOL_DB_SERIES` (series → tag default) plus optional `EXASOL_VERSION` override for the exact tag |
| `CB_SELF TO '<docker-gateway>:<mapped-port>'` | `CB_SELF TO '<Harness::connect_back_sql_address()>'`: testcontainers mode → `<container-eth0-ip>:8563` (container `eth0`, no NAT); external mode (`EXASOL_HOST` set) → `<host>:<db_port>` (the cluster SQL endpoint the harness already knows) |
| CI matrix uses `--skip connect_back` on 2025.x | CI matrix runs connect-back on every version; sets `EXASOL_DB_SERIES` per entry |

## Implementation Tasks

1. **Version-series plumbing in the harness**
   1.1 Add Cargo features `db-2025-1`, `db-2025-2`, `db-2026-1` to `crates/it/Cargo.toml`, with `default = ["db-2026-1"]`; each feature is a capability declaration (no `cfg` gating of test bodies).
   1.2 Add `db_series()` to `crates/it/src/lib.rs`: read `EXASOL_DB_SERIES`; if unset, fall back to the compiled default feature; reject unknown values with a clear error.
   1.3 Map series → default image tag inside `db_tag()` (keep `EXASOL_VERSION` as an explicit override that wins over the series default).

2. **Reimplement `cluster_ip()` via network interface detection** [expert]
   2.1 Rewrite `cluster_ip()` in `crates/exa-udf-runtime/src/rowset.rs` to return the local node's primary IPv4 (first non-loopback IPv4 of the UDF process, e.g. `eth0`) via `libc::getifaddrs`, instead of calling `parse_cluster_ip(&self.endpoint)`. Handle the `getifaddrs` linked-list traversal and `AF_INET` filtering carefully; skip loopback (`127.0.0.0/8`) and any interface without a valid `sockaddr_in`.
   2.2 Remove `parse_cluster_ip()` from `crates/exa-udf-runtime/src/artifact.rs` once `cluster_ip()` no longer references it (dead code).

3. **Connect-back address fix**
   3.1 Add a `connect_back_sql_address()` method to `Harness` in `crates/it/src/lib.rs` that returns the correct `CB_SELF` TCP address per deployment mode: in testcontainers mode (`self._container.is_some()`) return `format!("{}:8563", self.container_inner_ip().await?)` (container `eth0`, bypassing NAT — the actual crash fix); in external mode (`EXASOL_HOST` set, `self._container.is_none()`) return `format!("{}:{}", self.host, self.db_port)` (the cluster SQL endpoint the harness already carries, reachable from within the node). No new env var. Note: in testcontainers mode `host:db_port` is the NAT-mapped ephemeral host port (the original broken path), so the eth0 path is required there; in external mode there is no container to `docker exec`, so `container_inner_ip()` would fail and `host:db_port` is the only correct address.
   3.2 Change `db_roundtrip.rs` connect-back scenarios to create `CB_SELF TO '<connect_back_sql_address()>'` instead of the gateway/NAT address.
   3.3 Remove `container_connect_back_address()` from `crates/it/src/lib.rs` and its callers (dead once `connect_back_sql_address()` replaces it).

4. **Turn connect-back scenarios into hard assertions** [expert]
   4.1 Rewrite `connect_back_udf_queries_and_emits` to assert `42` as a hard result and assert the parent session is still alive afterwards (no SIGABRT match arms).
   4.2 Rewrite `connect_back_dml_inserts_visible_via_exapump` to assert `exapump` returns exactly `10,20,30` as a hard assertion.
   4.3 Rewrite `connect_back_cluster_ip_emits_node_ip` to assert the emitted string is a valid IPv4 address as a hard assertion on every series — no severity branch, no `ipc://`-error path.
   4.4 Remove `is_known_sigabrt_failure()`, `is_known_ipc_transport_failure()`, the SIGABRT match arms, and the ADR-015 ordering comment. Re-evaluate whether connect-back must still run last now that it no longer crashes the session; reorder if it simplifies the flow.

5. **Strip SIGABRT / ADR-015 from runtime + comments**
   5.1 Remove SIGABRT-related comments from `crates/exa-udf-runtime/src/connect_back.rs` (if any remain) and any docstrings referencing the crash.
   5.2 Grep the workspace for `SIGABRT`, `ADR-015`, `signal 6`, `Part:44`, `known-failing` in code/comments and remove stale references.

6. **CI matrix update** [expert]
   6.1 Keep `build-artifacts` compiling `it-runner` once with `--features integration,db-2026-1`.
   6.2 Update `.github/workflows/ci.yml` integration matrix: remove `--skip connect_back`; add `EXASOL_DB_SERIES` per entry (`2025-1`, `2025-2`, `2026-1`); keep `EXASOL_VERSION` as the exact tag.
   6.3 Confirm the `it-runner` invocation passes no per-version compile flags (runtime env only).

7. **Spec record alignment (decision-log)**
   7.1 Note in the plan decision-log that ADR-013/ADR-015 are superseded; the permanent `specs/decision-log.md` cleanup happens at `speq record` time, not here.

8. **Verification**
   8.1 Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`.
   8.2 Run the integration suite locally for each series via `EXASOL_DB_SERIES`.
   8.3 Regression: confirm the pre-existing scenarios that do NOT touch connect-back still pass unchanged after the `cluster_ip()` rewrite, the `connect_back_sql_address()` change, the SIGABRT cleanup, and the CI update — `sanity_select_one`, `scalar_double_returns_42`, `set_filter_emits_positive_only`, `json_parse_extracts_name`, `udf_error_surfaces_prefix`, `single_call_default_output_columns_roundtrip`, `single_call_unimplemented_returns_undefined`. None of these touch connect-back, so they are structurally safe; this step verifies no incidental regression.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 (harness version plumbing), Task 2 (cluster_ip rewrite), Task 5 (SIGABRT comment cleanup) |
| Group B | Task 3 (address fix), Task 4 (hard assertions) |
| Group C | Task 6 (CI), Task 7 (decision-log) |

Sequential dependencies:
- Group A → Group B (assertions/address use `db_series()` + the new `cluster_ip()`)
- Group B → Group C (CI matrix mirrors the harness env contract)
- Group C → Task 8 (verify last)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/exa-udf-runtime/src/artifact.rs` `parse_cluster_ip()` | `cluster_ip()` now reads the network interface; the endpoint-parsing path is dead |
| Method | `crates/it/src/lib.rs` `container_connect_back_address()` | Gateway/NAT address replaced by the new deployment-mode-aware `connect_back_sql_address()` |
| Function | `crates/it/tests/db_roundtrip.rs` `is_known_sigabrt_failure()` | No known SIGABRT failure remains |
| Function | `crates/it/tests/db_roundtrip.rs` `is_known_ipc_transport_failure()` | No longer needed; `cluster_ip()` is a hard IPv4 assertion on every series |
| Comment block | `crates/it/tests/db_roundtrip.rs` ADR-015 ordering comment | Connect-back no longer crashes the session |
| Spec scenario | `integration/connect-back` "reaches a routable database endpoint without crashing" | Folded into the query/DML hard-assertion scenarios |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| cluster_ip UDF emits the node IP | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_cluster_ip_emits_node_ip` (hard IPv4 assertion, every series) |
| Connect-back UDF queries the database and emits the result | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` |
| Connect-back DML UDF inserts rows and data is visible externally | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_dml_inserts_visible_via_exapump` |
| Harness starts Exasol and connects | Integration | `crates/it/tests/db_roundtrip.rs` | `sanity_select_one` (within `db_roundtrip_all_scenarios`) |
| Integration harness runs against the selected database version | Integration | `crates/it/tests/db_roundtrip.rs` | `db_series_selects_version_at_runtime` (new) |
| Pre-existing scenarios remain passing (regression) | Integration | `crates/it/tests/db_roundtrip.rs` | `sanity_select_one`, `scalar_double_returns_42`, `set_filter_emits_positive_only`, `json_parse_extracts_name`, `udf_error_surfaces_prefix`, `single_call_default_output_columns_roundtrip`, `single_call_unimplemented_returns_undefined` (none touch connect-back; MUST remain passing) |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| integration/connect-back | `EXASOL_DB_SERIES=2026-1 cargo test -p it --features integration -- --nocapture connect_back` | `connect_back_query ok`, `connect_back_dml ok`, `connect_back_cluster_ip ok`; no SIGABRT/known-failing lines |
| integration/connect-back | `EXASOL_DB_SERIES=2025-1 cargo test -p it --features integration -- --nocapture connect_back` | Connect-back query/DML pass; cluster_ip emits a valid IPv4 (hard assertion) |
| integration/db-roundtrip | `EXASOL_DB_SERIES=2026-1 cargo test -p it --features integration -- --nocapture` | All scenarios pass against the `2026.1.0` image |
| integration/db-roundtrip | `EXASOL_DB_SERIES=bogus cargo test -p it --features integration` | Fails fast with a clear "unrecognised EXASOL_DB_SERIES" error |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test (unit) | `cargo test --exclude it --workspace` | 0 failures |
| Test (integration) | `EXASOL_DB_SERIES=2026-1 cargo test -p it --features integration` | 0 failures |
| Regression | `EXASOL_DB_SERIES=2026-1 cargo test -p it --features integration` | All pre-existing scenarios pass unchanged (`sanity_select_one`, `scalar_double_returns_42`, `set_filter_emits_positive_only`, `json_parse_extracts_name`, `udf_error_surfaces_prefix`, `single_call_default_output_columns_roundtrip`, `single_call_unimplemented_returns_undefined`) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |

## Decision Log

- ADR-013 and ADR-015 are superseded by this plan. The SIGABRT crash was caused by routing the connect-back through the Docker host gateway/NAT path, not a database bug. Connecting over TCP to the node's own SQL endpoint (`cluster_ip():8563` in testcontainers mode; `host:db_port` in external mode) eliminates the crash entirely. Permanent cleanup of `specs/decision-log.md` happens at `speq record` time.
