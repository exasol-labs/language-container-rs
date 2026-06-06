# Decision Log: fix-connect-back-external-client

Date: 2026-06-06

## Interview

**Q:** Query result API — does the `ExaConnection` surface change?
**A:** No. Keep Arrow — `query_arrow() -> Vec<RecordBatch>`. exarrow-rs already bridges Arrow to Exasol's native protocol. No API change.

**Q:** Which transport protocol for connect-back?
**A:** Native protocol (exarrow-rs default, no `transport=` override). A spike confirmed native connects and executes SQL as a plain external client against `exasol/docker-db:2026.1.0`. The WebSocket feature is not compiled in by default and is not needed.

**Q:** What does "CoreDB" mean in the constraint "do not implement in combination with CoreDB"?
**A:** "CoreDB" = the Exasol internal connect-back proxy at `127.0.0.1:8563` inside the UDF network namespace. Connecting there triggers a SIGABRT on 2026.1.0 (server-side bug: Exasol tries to associate Part:44 with Part:40 and crashes). The fix is to connect to the address from `CREATE CONNECTION ... TO '<external-address>'`, exactly like PyExasol.

**Q:** How is the address obtained, and is the UDF portable?
**A:** strata-rs pattern — the operator creates `CREATE CONNECTION CB_SELF TO '<cluster-ip>:8563' USER '...' IDENTIFIED BY '...'`. The UDF uses a generic `%connection CB_SELF` directive. The runtime sends `MT_IMPORT` with `conn_name`; Exasol responds with the ConnInfo for the named connection; the runtime connects to `conn_info.address` as a regular external client. The IT harness computes the Docker gateway address (`container_connect_back_address()`) and stores it in `CREATE CONNECTION CB_SELF`. The address is operator-configured, not auto-discovered, so the artifact stays portable.

**Q:** Does the gateway-address approach (added after the archived plan) fix the SIGABRT?
**A:** This was the key empirical question this plan was tasked to answer by re-running the integration suite.

## Design Decisions

### [1] Connect-back is always a new external client session and a new transaction

- **Decision:** The runtime opens connect-back as an ordinary external client login to the named connection's `address`/`user`/`password`, establishing a new session and a new transaction. The `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) exchange is treated purely as metadata retrieval, equivalent to PyExasol's `exa.get_connection(NAME)`.
- **Alternatives:** Attempt to join/share the invoking query's session or transaction; treat the named connection as an internal proxy token at loopback/eth0 `:8563`.
- **Rationale:** The Exasol core cannot share the invoking query's transaction with a language-container UDF. The reference SLCs (Python/Java) all open an independent external client. The internal-proxy interpretation is what caused the original `2026.1.0` SIGABRT.
- **Promotes to ADR:** yes

### [2] Empirical re-verification: the Docker-host-gateway address does NOT fix the SIGABRT

- **Decision:** Record, from a fresh `2026-06-06` integration run, that connect-back still SIGABRTs the invoking session on `exasol/docker-db:2026.latest` even with the Docker-host-gateway external-client address. 6/8 scenarios pass; both connect-back scenarios fail with `peer closed connection without sending TLS close_notify`, and the container log shows `child <pid> (Part:40 Node:0 exasql) terminated with signal 6. (core dumped)` immediately after `Part:44` (the connect-back session) starts. `2026.latest` and `2026.1.0` share image id `b81d80f63d10`, so no patched image exists.
- **Alternatives:** Assume the gateway fix resolved the crash (the original hypothesis behind commit `7de7357`); declare the prior SIGABRT comments stale and remove them.
- **Rationale:** Direct evidence contradicts the hypothesis. The crash is server-side, triggered by the core spawning a connect-back session for a container UDF, independent of connect-back address or transport. The SLC implementation is correct and cannot work around it.
- **Promotes to ADR:** yes

### [3] Native binary protocol only; drop the exarrow-rs `websocket` feature flag

- **Decision:** Remove `features = ["websocket"]` from the `exarrow-rs` dependency in `crates/exa-udf-runtime/Cargo.toml`. The DSN already emits no `transport=` override, so the native default is used.
- **Alternatives:** Keep the WebSocket feature compiled in for possible future use.
- **Rationale:** Native is the only transport the runtime uses; the WebSocket transport was only ever a workaround hypothesis for the address-misuse SIGABRT, now disproven. Removing it shrinks the dependency surface and matches the user's native-only mandate.
- **Promotes to ADR:** no

### [4] Keep connect-back integration scenarios as honest known-failing gates

- **Decision:** Retain `connect_back_udf_queries_and_emits` and `connect_back_dml_inserts_visible_via_exapump` in the suite, failing visibly on `2026.latest` with diagnostics dumped, rather than deleting them or asserting a false pass.
- **Alternatives:** Mark them `#[ignore]`; delete them; make them assert the failure as success.
- **Rationale:** They form a regression net that will turn green automatically once Exasol ships a patched image, while honestly surfacing the current blocker.
- **Promotes to ADR:** no

### [5] Pin the integration image to `2026.latest`

- **Decision:** Set `DB_TAG = "2026.latest"` in `crates/it/src/lib.rs`.
- **Alternatives:** Keep `2026.1.0`.
- **Rationale:** Project CLAUDE.md mandates `2026.latest`. The tags resolve to the same image id today, so behaviour is unchanged, but the pin tracks future patched builds automatically.
- **Promotes to ADR:** no

### [6] Comments are accurate, not stale — refresh provenance instead of removing

- **Decision:** Update the SIGABRT comments in `db_roundtrip.rs` and `it/src/lib.rs` to cite the `2026-06-06` gateway re-verification and the new ADR, instead of removing them as "stale".
- **Alternatives:** Remove the SIGABRT comments on the assumption the gateway fix resolved the crash.
- **Rationale:** The re-run proves the comments still describe real behaviour; only their reference to "decision [15]" and their provenance needed updating.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
