# Plan: add-connection-api

## Summary

Replace the `exa()`/`exa_named()`/`exa_connect()` connect-back surface with three explicit, composable `UdfContext` methods — `cluster_ip()`, `connection(name)`, and `connect_back(&ConnectionObject)` — backed by a new public `ConnectionObject` SDK type so UDF authors can read raw credentials, target the cluster node directly, or connect to foreign systems with any driver.

## Design

### Context

The current connect-back API (`exa`, `exa_named`, `exa_connect`, `ConnectBackOptions`) couples credential retrieval and session opening into opaque calls and hides the credential payload from the author. UDF authors need three separable capabilities: learn the originating cluster node IP, fetch raw credentials of any named `CONNECTION` object, and open a live Exasol session — composable so an author can also drive a foreign system with their own driver.

- **Goals** — expose `cluster_ip()`, `connection(name) -> ConnectionObject`, and `connect_back(&ConnectionObject) -> Box<dyn ExaConnection>`; make `ConnectionObject` a first-class public SDK type; keep credential retrieval (`connection`) separate from session opening (`connect_back`).
- **Non-Goals** — no change to the wire protocol (`MT_IMPORT`/`PB_IMPORT_CONNECTION_INFORMATION` already exist); no JIT; no WebSocket transport; no automatic credential caching of the default handshake connection (the lazy default `exa()` connection is removed).

### Decision

Split the connect-back surface into one credential type and three methods. `connection(name)` performs the on-demand `MT_IMPORT` exchange already proven during `run_batch` and returns raw `ConnectionObject` fields. `connect_back(&ConnectionObject)` builds the DSN and opens an `exarrow-rs` session, returning an owned `Box<dyn ExaConnection>`. `cluster_ip()` parses the ZMQ endpoint string with no network call.

#### Architecture

```
                        UdfContext (SDK trait, connect-back feature)
                        ┌───────────────────────────────────────────┐
                        │ cluster_ip() -> String                     │
                        │ connection(name) -> ConnectionObject       │
                        │ connect_back(&ConnectionObject) -> Box<dyn> │
                        └───────────────────────────────────────────┘
                                          ▲ impl
   args[1] endpoint                       │
   tcp://<ip>:<port> ──▶ Runtime ──▶ dispatch ──▶ run_batch ──▶ HostContextBridge
                          (stores)     (passes)    (passes)        │
                                                                   │ connection(name)
                                                                   ▼
                                                conn_requester closure ──▶ MT_IMPORT ──▶ ConnInfo
                                                                   │ map
                                                                   ▼ ConnectionObject
                                                                   │ connect_back(&obj)
                                                                   ▼
                                                open_connection(addr,user,pass) ──▶ CONNECT_BACK_RT ──▶ Box<dyn ExaConnection>
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Public DTO vs internal wire type | `ConnectionObject` (SDK) ↔ `ConnInfo` (protocol) | Keeps transport types out of the author-facing API (ADR-016) |
| Parse-not-fetch | `cluster_ip()` parses `args[1]` | The endpoint already names the node; no round-trip needed (ADR-017) |
| Synchronous request/reply during a blocked dispatch loop | `connection(name)` MT_IMPORT in `run_batch` | The outer loop is blocked awaiting the UDF return, so the ZMQ socket is idle and safe (ADR-018) |
| Separate retrieval from connection | `connection` vs `connect_back` | Author may use credentials with a foreign driver, or pair them with `cluster_ip` |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| `ConnectionObject` is a public SDK type; `ConnInfo` stays internal | Re-export `ConnInfo` from the SDK | A public DTO keeps `exa-zmq-protocol` out of the SDK's public surface and lets the field set evolve independently |
| `cluster_ip()` returns raw `<node_ip>`, no port | Append `:8563` | Authors choose the port; raw IP composes with `connection` credentials and any target port |
| `connection(name)` sends MT_IMPORT during `run_batch` | Require all names declared in `%connection` header at handshake | The protocol already parses `MT_IMPORT` in any phase and the existing closure proves the run-phase exchange is safe |
| Remove `exa`/`exa_named`/`exa_connect`/`ConnectBackOptions` | Keep them as deprecated aliases | 0.x library; the new three-method surface fully supersedes them and a clean break avoids two parallel APIs |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-sdk | CHANGED | `sdk/udf-sdk/spec.md` |
| runtime/host-dispatch | CHANGED | `runtime/host-dispatch/spec.md` |
| integration/db-roundtrip | CHANGED | `integration/db-roundtrip/spec.md` |

## Dependencies

- `exarrow-rs` (already a runtime dependency behind `connect-back`)
- `exa-zmq-protocol` `import_connection_request` / `HostEvent::ConnInfo` (already present)

## Migration

| Current | New |
|---------|-----|
| `ctx.exa()` (lazy default connection) | `ctx.connect_back(&ctx.connection(<name>)?)` — no implicit default connection |
| `ctx.exa_named(name)` | `let c = ctx.connection(name)?; ctx.connect_back(&c)?` |
| `ctx.exa_connect(ConnectBackOptions::Explicit { dsn, user, password })` | `ctx.connect_back(&ConnectionObject { kind, address, user, password })` |
| `ConnectBackOptions` enum | `ConnectionObject` struct |

## Implementation Tasks

1. **SDK: `ConnectionObject` and `ExaConnection`**
   1. Add `pub struct ConnectionObject { kind, address, user, password }` to `crates/exasol-udf-sdk/src/connect_back.rs`; remove `ConnectBackOptions`.
   2. Keep `ExaConnection` trait (`query_arrow`, `execute`); update module docs to drop `ConnectBackOptions`.
   3. Update `crates/exasol-udf-sdk/src/lib.rs` re-exports: export `ConnectionObject, ExaConnection`; drop `ConnectBackOptions`.

2. **SDK: `UdfContext` method surface**
   1. In `crates/exasol-udf-sdk/src/context.rs`, replace `exa`/`exa_named`/`exa_connect` with `cluster_ip(&self) -> Result<String, UdfError>`, `connection(&self, name: &str) -> Result<ConnectionObject, UdfError>`, and `connect_back(&mut self, conn: &ConnectionObject) -> Result<Box<dyn ExaConnection>, UdfError>`, each defaulting to `UdfError::Unimplemented`.

3. **Runtime: endpoint propagation for `cluster_ip`**
   1. Thread the ZMQ endpoint from `Runtime` → `dispatch::run_udf` → `run_batch` → `HostContextBridge`.
   2. Add a `parse_cluster_ip(endpoint: &str) -> Option<String>` helper (strip `tcp://`, split on `:`, take host) in `crates/exa-udf-runtime/src/artifact.rs` (or a small new module). [expert]

4. **Runtime: `HostContextBridge` connect-back rewiring** [expert]
   1. Replace the `exa()` bridge method and its lazy-default `conn`/`conn_info` caching with `cluster_ip()`, `connection(name)`, and `connect_back(&ConnectionObject)`.
   2. Make `connection(name)` send MT_IMPORT via a `name`-parameterised requester (generalise the existing `conn_requester` closure to take the connection name) and map `ConnInfo` → `ConnectionObject`.
   3. Make `connect_back(&ConnectionObject)` map `ConnectionObject` → `ConnInfo`, call `open_connection`, and return the owned `Box<dyn ExaConnection>` (drop the single-connection cache; authors own their boxes).
   4. Update `run_batch` in `crates/exa-udf-runtime/src/dispatch.rs`: pass the endpoint and a name-parameterised MT_IMPORT closure; remove the proactive-handshake-credentials priority path.

5. **Runtime: handshake conn_info cleanup**
   1. Remove the now-unused proactive `conn_info` seeding in `dispatch.rs`/`lib.rs` handshake (no implicit default connection). Verify `meta.conn_info` is still set where the protocol layer expects it, or remove if fully dead.

6. **Tests: SDK**
   1. Rewrite `crates/exasol-udf-sdk/tests/connect_back.rs` to assert `ConnectionObject` fields and the three-method surface.
   2. Update `crates/exasol-udf-sdk/tests/feature_gate.rs` comment/assertions for the new absent method names.

7. **Tests: runtime**
   1. Rewrite `crates/exa-udf-runtime/tests/connect_back.rs` mock-based tests for `connect_back(&ConnectionObject)` and connection reuse semantics (now author-owned boxes).
   2. Add a `parse_cluster_ip` unit test (pure parsing) covering `tcp://10.0.0.5:6583 → 10.0.0.5` and a malformed-endpoint error.
   3. Add an integration test driving `connection(name)` over a mock DB that replies to MT_IMPORT with a `connection_information_rep`, asserting the returned `ConnectionObject` fields.

8. **Docs**
   1. Update example UDFs and any SDK rustdoc that referenced `exa()`/`ConnectBackOptions`.

9. **Test UDFs: migrate to new API** [expert]
   1. Rewrite `test-udfs/connect-back-query/src/lib.rs`: replace `ctx.exa()?` with `let c = ctx.connection("CB_SELF")?; ctx.connect_back(&c)?`.
   2. Rewrite `test-udfs/connect-back-insert/src/lib.rs`: same migration; UDF must call `ctx.connection("CB_SELF")` once and reuse the returned `ConnectionObject` across both `execute` calls in the row loop (or re-fetch each time — document the choice).
   3. Add `test-udfs/connect-back-cluster-ip/` — a new scalar UDF that calls `ctx.cluster_ip()` and emits the raw node IP string. This scenario does NOT open a connect-back session, so it is not blocked by ADR-015 and MUST pass on 2026.latest.

10. **Integration harness: live Docker scenarios** [expert]
    1. Upload `libconnect_back_cluster_ip.so` in `db_roundtrip_all_scenarios` alongside the existing artifacts.
    2. Add scenario `connect_back_cluster_ip_emits_node_ip` (hard assert, not KNOWN_FAILING):
       - Create a SCALAR SCRIPT for the cluster-IP UDF.
       - `SELECT TO_CHAR(connect_back_cluster_ip())` and assert the result is a non-empty string matching an IPv4 pattern.
       - This scenario MUST pass on 2026.latest because `cluster_ip()` is pure parsing — no connect-back session is opened, so no SIGABRT.
    3. Keep existing `connect_back_udf_queries_and_emits` and `connect_back_dml_inserts_visible_via_exapump` as KNOWN_FAILING (ADR-015); update their inline UDF scripts to use the new three-method API (they use the migrated test-UDF artifacts from Task 9).
    4. Add `integration/db-roundtrip` spec delta (`integration/db-roundtrip/spec.md` in this plan) for the new cluster-IP scenario and the updated connect-back scenarios.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1, Task 2 (SDK types + trait) |
| Group B | Task 3 (endpoint plumbing + parse helper) |
| Group C | Task 4, Task 5 (bridge rewiring + cleanup) |
| Group D | Task 6, Task 7, Task 8 (tests + docs) |
| Group E | Task 9, Task 10 (test UDFs + live Docker harness) |

Sequential dependencies:
- Group A → Group C (bridge depends on the new SDK trait + `ConnectionObject`)
- Group B → Group C (bridge `cluster_ip` needs the propagated endpoint + parse helper)
- Group C → Group D (tests exercise the rewired bridge)
- Group C → Group E (test UDFs compile against the rewired bridge; live harness needs compiled artifacts)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Enum | `crates/exasol-udf-sdk/src/connect_back.rs` `ConnectBackOptions` | Superseded by `ConnectionObject` |
| Methods | `crates/exasol-udf-sdk/src/context.rs` `exa`, `exa_named`, `exa_connect` | Replaced by `cluster_ip`, `connection`, `connect_back` |
| Re-export | `crates/exasol-udf-sdk/src/lib.rs` `ConnectBackOptions` | Type removed |
| Field/path | `HostContextBridge` lazy-default `conn` cache + proactive `conn_info` priority path | No implicit default connection; authors own returned boxes |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name | Pass on 2026.latest? |
|----------|-----------|---------------|-----------|----------------------|
| sdk/udf-sdk — ConnectionObject is a public connect-back SDK type | compile-time | `crates/exasol-udf-sdk/tests/connect_back.rs` | `connection_object_exposes_fields` | ✓ |
| sdk/udf-sdk — ExaConnection trait is defined behind the connect-back feature | compile-time | `crates/exasol-udf-sdk/tests/connect_back.rs` | `exa_connection_trait_has_query_and_execute` | ✓ |
| sdk/udf-sdk — UdfContext connect-back methods are absent without the feature | compile-time | `crates/exasol-udf-sdk/tests/feature_gate.rs` | `connect_back_methods_absent_without_feature` | ✓ |
| sdk/udf-sdk — UdfContext exposes connect-back methods with the feature | compile-time | `crates/exasol-udf-sdk/tests/connect_back.rs` | `udfcontext_exposes_cluster_ip_connection_connect_back` | ✓ |
| sdk/udf-sdk — connect_back accepts a caller-built ConnectionObject for a foreign target | compile-time | `crates/exasol-udf-sdk/tests/connect_back.rs` | `connect_back_accepts_caller_built_object` | ✓ |
| runtime/host-dispatch — cluster_ip is parsed from the ZMQ endpoint without a network call | unit | `crates/exa-udf-runtime/src/artifact.rs` | `parse_cluster_ip_strips_scheme_and_port` | ✓ |
| runtime/host-dispatch — connection fetches named-connection credentials via on-demand MT_IMPORT | mock | `crates/exa-udf-runtime/tests/connect_back.rs` | `connection_fetches_credentials_via_mt_import` | ✓ |
| runtime/host-dispatch — Connect-back opens a connection from a ConnectionObject | mock | `crates/exa-udf-runtime/tests/connect_back.rs` | `connect_back_opens_from_connection_object` | ✓ |
| runtime/host-dispatch — Connect-back query returns Arrow batches to the UDF | mock | `crates/exa-udf-runtime/tests/connect_back.rs` | `query_arrow_returns_record_batches` | ✓ |
| runtime/host-dispatch — Connect-back connects to the named connection address like an external client | mock | `crates/exa-udf-runtime/tests/connect_back.rs` | `connect_back_dsn_targets_address_as_external_client` | ✓ |
| runtime/host-dispatch — Connect-back named connection makes the UDF portable across clusters | mock | `crates/exa-udf-runtime/tests/connect_back.rs` | `connect_back_dsn_built_only_from_connection_object` | ✓ |
| integration/db-roundtrip — cluster_ip UDF emits the node IP (live Docker, hard assert) | live Docker | `crates/it/tests/db_roundtrip.rs` | `connect_back_cluster_ip_emits_node_ip` | ✓ (no session opened) |
| integration/db-roundtrip — connect-back UDF queries via ConnectionObject (live Docker) | live Docker | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` | KNOWN_FAILING ADR-015 |
| integration/db-roundtrip — connect-back DML UDF inserts via ConnectionObject (live Docker) | live Docker | `crates/it/tests/db_roundtrip.rs` | `connect_back_dml_inserts_visible_via_exapump` | KNOWN_FAILING ADR-015 |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-sdk | `cargo test -p exasol-udf-sdk --features connect-back` | All connect-back + feature-gate tests pass |
| runtime/host-dispatch | `cargo +1.91 test -p exa-udf-runtime --features connect-back` | `parse_cluster_ip` + connect-back mock tests pass |
| integration/db-roundtrip | `cargo +1.91 test -p it --features integration -- --nocapture` | `cluster_ip` scenario PASSES; connect-back scenarios log KNOWN_FAILING (ADR-015) with SIGABRT signature; suite exits 0 |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo +1.91 build --release` | Exit 0 |
| Test | `cargo +1.91 test` | 0 failures |
| Lint | `cargo +1.91 clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
| Live Docker | `cargo +1.91 test -p it --features integration -- --nocapture` | cluster_ip PASS; CB scenarios KNOWN_FAILING; exit 0 |
