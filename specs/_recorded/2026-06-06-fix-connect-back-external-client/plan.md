# Plan: fix-connect-back-external-client

## Summary

Align Rust-UDF connect-back fully with the PyExasol / strata-rs pattern — a new external client session opened to the address/user/password of a named `CREATE CONNECTION` object — and document, via fresh empirical evidence, that the connect-back round-trip remains blocked by a server-side SIGABRT on `exasol/docker-db:2026.latest` (identical image to `2026.1.0`). This plan ships portability, transport, and documentation hardening plus an honest known-failing gate for the connect-back integration scenarios; it does not attempt to work around the upstream core bug.

## Design

### Context

Connect-back lets a UDF query the database from inside its `run`. The reference SLCs (Python/Java) and strata-rs do this by reading connection metadata (`exa.get_connection(NAME)`) and opening a brand-new external client connection to that address — a new session, a new transaction — because the Exasol core cannot share the invoking query's transaction with a language-container UDF.

The Rust runtime already implements exactly this: it fetches credentials via `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`, a pure metadata fetch equivalent to `exa.get_connection`), then connects to `conn_info.address` as an external client over the exarrow-rs native protocol with cert validation disabled. ADR-012 and ADR-013 already record this direction. The open empirical question was whether the Docker-host-gateway address (added in commit `7de7357`, after the archived plan's verification) lets the connect-back round-trip succeed on the available image.

This plan answers that question with a fresh integration run and squares the codebase, specs, and docs with the answer.

- **Goals**
  - Confirm empirically whether connect-back works on the available image with the gateway-address approach.
  - Make the connect-back path unambiguously an external-client, new-session, new-transaction operation in code comments, specs, and a new README section.
  - Guarantee UDF portability: address comes only from the named `CREATE CONNECTION`; the artifact hardcodes nothing.
  - Remove the now-confirmed-unnecessary `websocket` exarrow-rs feature flag (native is the only transport used).
  - Pin the integration image to `2026.latest` per project rules.
- **Non-Goals**
  - Changing the `ExaConnection` trait API (`query_arrow`, `execute` are frozen).
  - Implementing or enabling the WebSocket connect-back transport.
  - Working around the server-side SIGABRT (it is an upstream core defect, not fixable in the SLC).
  - Any internal-proxy / CoreDB (`127.0.0.1:8563`) connect-back path.

### Decision

#### Empirical result (key finding)

A full integration run on `2026-06-06` (`cargo +1.91 test -p it --features integration`) produced:

- 6 / 8 scenarios PASS (scalar, set, json, udf-error, both single-call).
- Both connect-back scenarios FAIL with `peer closed connection without sending TLS close_notify` on the **outer** session.
- Container log signature: `child <pid> (Part:40 Node:0 exasql) terminated with signal 6. (core dumped)` immediately after `Part:44` (the connect-back session process) is spawned.

This reproduces archived decision [15] **with the Docker-host-gateway address**. The gateway-address change did not resolve the crash. `docker images` confirms `2026.latest` and `2026.1.0` share image id `b81d80f63d10`, so no patched image is available.

Conclusion: the runtime connect-back implementation is correct and matches the Python pattern; the round-trip is blocked solely by a server-side core defect on `2026.1.0`/`2026.latest`.

#### Architecture

```
UDF run()                Host runtime                         Exasol core
  ctx.exa()  ───────────▶ MT_IMPORT(PB_IMPORT_CONNECTION_INFORMATION, name=CB_SELF)
                          ◀───── connection_information_rep {address,user,password}
                          build_dsn(): exasol://user:pw@address?validateservercertificate=0
                          exarrow-rs (native) ──── new external login ───▶ spawns Part:44
                                                                            │
   (correct path)                                                          ▼
                                              2026.latest BUG: Part:40 SIGABRT (signal 6)
```

The metadata fetch (`MT_IMPORT`) mirrors Python's `exa.get_connection(NAME)`; the subsequent connect is an ordinary external client login. Nothing in the SLC asks the core to associate the new session with the invoking session — the association and crash are entirely server-side.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Named-connection metadata fetch then external-client connect | `dispatch.rs` `conn_requester` + `connect_back.rs::open_connection` | Matches PyExasol/strata-rs; keeps the artifact portable |
| New session / new transaction per connect-back | `connect_back.rs` | Core cannot share the invoking query's transaction with a container UDF |
| Native-only DSN (no `transport=`) | `connect_back.rs::build_dsn` | Native is the default and only used transport; WebSocket unneeded |
| Known-failing gate with diagnostics | `db_roundtrip.rs` | Surfaces the upstream SIGABRT honestly instead of masking or asserting a false pass |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Keep connect-back as a new external session/transaction; document explicitly | Attempt to join the invoking transaction | Core cannot share a transaction with a container UDF; matches reference SLCs |
| Document the SIGABRT as an unresolved upstream blocker; keep connect-back scenarios as known-failing | Delete the connect-back scenarios; pretend they pass | Honest evidence; scenarios become a regression net once a patched image ships |
| Remove the `websocket` feature from the exarrow-rs dep | Leave the flag for "future use" | Dead config; native is the only transport; smaller dependency surface |
| Pin `DB_TAG` to `2026.latest` | Keep `2026.1.0` | Project rule mandates `2026.latest`; same image id today so behaviour is unchanged |
| Keep SIGABRT comments but update them to record the re-verified gateway result | Remove the comments as "stale" | The comments are accurate; only their provenance needs refreshing |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| runtime/host-dispatch | CHANGED | `specs/_plans/fix-connect-back-external-client/runtime/host-dispatch/spec.md` |
| sdk/udf-sdk | CHANGED | `specs/_plans/fix-connect-back-external-client/sdk/udf-sdk/spec.md` |
| integration/db-roundtrip | CHANGED | `specs/_plans/fix-connect-back-external-client/integration/db-roundtrip/spec.md` |

## Dependencies

- `exarrow-rs` (local, v0.12.5) — native feature only after this plan; `websocket` feature dropped from the runtime dependency.
- `exasol/docker-db:2026.latest` — Docker image for integration; identical to `2026.1.0` at time of writing.

## Implementation Tasks

1. Runtime transport hardening
   - [ ] 1.1 Remove `features = ["websocket"]` from the `exarrow-rs` dependency in `crates/exa-udf-runtime/Cargo.toml`; confirm `connect-back` still builds with the native default.
   - [ ] 1.2 Confirm `build_dsn` in `crates/exa-udf-runtime/src/connect_back.rs` emits no `transport=` override and update its doc comment to state new-session/new-transaction external-client semantics (no behaviour change).

2. Integration harness alignment
   - [ ] 2.1 Change `DB_TAG` in `crates/it/src/lib.rs` from `2026.1.0` to `2026.latest` per project rules.
   - [ ] 2.2 Update the `container_connect_back_address` doc comment and the connect-back comments in `crates/it/tests/db_roundtrip.rs` to record the `2026-06-06` re-verification: the Docker-host-gateway address still triggers the server-side SIGABRT; reference the new ADR instead of "decision [15]".
   - [ ] 2.3 Keep the two connect-back scenarios in the suite as known-failing, ensuring the harness still dumps connect-back diagnostics on failure; do not assert a false pass. [expert]

3. Documentation
   - [ ] 3.1 Create `README.md` with a connect-back section: the operator creates `CREATE CONNECTION <NAME> TO '<cluster-address>:8563' USER '...' IDENTIFIED BY '...'`; the UDF stays generic with `%connection <NAME>`; connect-back is always a new external session/transaction; note the `2026.latest` server-side SIGABRT blocker.
   - [ ] 3.2 Add ADR-014 (connect-back is always a new external session/transaction) and ADR-015 (gateway-address re-verification: SIGABRT persists on `2026.latest`, blocker unresolved) to `specs/decision-log.md`.

4. Verification
   - [ ] 4.1 Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo build --release`.
   - [ ] 4.2 Re-run the integration suite and confirm 6/8 pass and the two connect-back scenarios fail with the documented SIGABRT signature (no regressions in non-connect-back scenarios). [expert]

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1, 1.2, 2.1, 2.2, 3.1, 3.2 |
| Group B | 2.3 |
| Group C | 4.1, 4.2 |

Sequential dependencies:
- Group A → Group B → Group C (verification runs after edits; harness gate after comment/tag updates).

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Cargo feature | `crates/exa-udf-runtime/Cargo.toml` (`exarrow-rs` `features = ["websocket"]`) | Native is the only connect-back transport; WebSocket is never compiled in or used |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Connect-back connects to the named connection address like an external client | Unit | `crates/exa-udf-runtime/src/connect_back.rs` | `dsn_disables_cert_validation_and_carries_credentials` |
| Connect-back named connection makes the UDF portable across clusters | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` (CB_SELF + `%connection CB_SELF`, known-failing on `2026.latest`) |
| UdfContext exposes connect-back methods with the feature | Unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | connect-back trait/method presence tests |
| Harness starts Exasol and connects | Integration | `crates/it/tests/db_roundtrip.rs` | `sanity_select_one` (within `db_roundtrip_all_scenarios`) |
| Connect-back UDF queries the database and emits the result | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` |
| Connect-back DML UDF inserts rows and data is visible externally | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_dml_inserts_visible_via_exapump` |
| Connect-back UDF reaches a routable database endpoint without crashing the session | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` (asserts session survival; known-failing on `2026.latest`) |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| runtime/host-dispatch | `cargo +1.91 test -p it --features integration -- --nocapture` | Non-connect-back scenarios log `ok`; connect-back scenarios fail with `peer closed connection without sending TLS close_notify` and a `signal 6 (core dumped)` Part:40 line in the dumped docker logs |
| integration/db-roundtrip | `docker images exasol/docker-db` | `2026.latest` and `2026.1.0` resolve to the same image id, confirming no patched build is available |
| sdk/udf-sdk | `cargo test -p exasol-udf-sdk --features connect-back` | connect-back trait/method tests pass; `query_arrow`/`execute` signatures unchanged |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
