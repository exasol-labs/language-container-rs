# Plan: add-udfcontext-handshake-metadata

## Summary

Expose the database handshake metadata (`exascript_info`: session/statement/node/vm IDs plus the DB/script/user string fields) to UDF authors through defaulted, non-feature-gated `UdfContext` accessors that mirror `memory_limit()`, and remove the dead write-only `UdfMeta::conn_info` field left over from a pre-ADR-018 design.

## Design

### Context

The DB delivers a rich `exascript_info` handshake (`MT_INFO`), but UDF code links only `exasol-udf-sdk` and receives `&dyn UdfContext`, so it can only reach `memory_limit()` today. The remaining handshake fields are either decoded into the host-internal `UdfMeta` (and never bridged) or not decoded at all. Separately, `UdfMeta::conn_info` is a buffered credentials field that no production code reads — ADR-018 replaced it with on-demand per-name resolution via `MT_IMPORT`.

- **Goals** — surface `session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user` to UDF code; delete the dead `conn_info` buffering.
- **Non-Goals** — no new connect-back capability; no feature flags; no change to the live on-demand `MT_IMPORT` credential path (the `ConnInfo` type and `HostEvent::ConnInfo` stay).

### Decision

Follow the established `memory_limit()` accessor pattern exactly: each accessor is a provided (defaulted) trait method on `UdfContext` returning a neutral value, overridden by `HostContextBridge` to return the live `UdfMeta` field. `UdfMeta` gains the fields not yet decoded (`statement_id`, `database_name`, `database_version`, `script_schema`, `current_user`, `current_schema`, `scope_user`) in `from_pb`; the bridge is threaded those values at construction.

#### Architecture

```
exascript_info (proto)
        │ UdfMeta::from_pb
        ▼
     UdfMeta  ──(threaded at construction)──▶  HostContextBridge
        │                                            │ overrides
        │ (host-internal)                            ▼
        └───────────────────────────────▶  &dyn UdfContext (UDF code)
                                              defaulted accessors
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Defaulted trait accessor returning a neutral value | `UdfContext` in `exasol-udf-sdk` | Existing impls keep compiling without supplying the method |
| Host bridge override | `HostContextBridge` in `exa-udf-runtime` | Live value reaches UDF code without exposing host internals |
| Owned return across vtable | string/optional accessors | `arrow`/`TypeId`-style stability concerns: only owned `String`/`Option<String>` cross the `.so` boundary, not borrows |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Defaulted, non-feature-gated accessors | Feature-gate behind `connect-back` | Handshake metadata is plain DB context, not a connect-back capability (CLAUDE.md) |
| String accessors return owned `String` | Return `&str` borrowing from the bridge | Owned values cross the `.so` vtable boundary safely; no lifetime entanglement |
| Optional fields return `Option<String>` with `None` default | Empty string for absent optionals | Preserves the proto `optional` present/absent distinction |
| Delete `UdfMeta::conn_info`; keep `ConnInfo`/`HostEvent::ConnInfo` | Leave the dead field | Write-only state read nowhere; on-demand path is the single live credential path |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-sdk | CHANGED | `sdk/udf-sdk/spec.md` |
| runtime/dispatch-run-loop | CHANGED | `runtime/dispatch-run-loop/spec.md` |
| protocol/wire-protocol | CHANGED | `protocol/wire-protocol/spec.md` |

## Implementation Tasks

1. **Extend `UdfMeta` with the undecoded handshake fields** — add `statement_id: u32`, `database_name: String`, `database_version: String`, `script_schema: String`, `current_user: Option<String>`, `current_schema: Option<String>`, `scope_user: Option<String>` to the struct and map them in `UdfMeta::from_pb` from `ExascriptInfo` (`crates/exa-zmq-protocol/src/meta.rs`). `session_id`, `node_id`, `node_count`, `vm_id`, `script_name` already exist. [expert]
2. **Add the defaulted `UdfContext` accessors** — `session_id`, `statement_id`, `node_id`, `node_count`, `vm_id` (numeric, default `0`), `database_name`, `database_version`, `script_name`, `script_schema` (owned `String`, default empty), `current_user`, `current_schema`, `scope_user` (`Option<String>`, default `None`); model on the existing `memory_limit()` default (`crates/exasol-udf-sdk/src/context.rs`). [expert]
3. **Thread the new fields into `HostContextBridge` and override the accessors** — extend the bridge struct + `new`/`with_connection` constructors to carry the handshake values from `UdfMeta`, and implement the overrides on the `UdfContext for HostContextBridge` impl (`crates/exa-udf-runtime/src/rowset.rs`); update the construction sites in `crates/exa-udf-runtime/src/dispatch.rs` (and any single-call construction site) to pass the `UdfMeta` values. [expert]
4. **Remove the dead `conn_info` handshake buffering** — delete `conn_info: Option<ConnInfo>` from `UdfMeta` and the `conn_info: None` initialiser in `from_pb` (`meta.rs`); remove the `let mut conn_info = None;`, `m.conn_info = conn_info.take();`, and `HostEvent::ConnInfo(ci) => conn_info = Some(ci)` lines from the handshake loop (`crates/exa-udf-runtime/src/lib.rs`). Keep `ConnInfo` and `HostEvent::ConnInfo`. [expert]
5. **Add SDK default-value unit tests** — assert each defaulted accessor returns its neutral value on a context that does not override (model on `default_memory_limit_is_zero`, `crates/exasol-udf-sdk/src/context.rs`).
6. **Add bridge-override unit tests** — assert the bridge returns the exact `UdfMeta` values, including `Some`/`None` for the optionals (model on `bridge_returns_memory_limit`, `crates/exa-udf-runtime/src/rowset.rs`).
7. **Add a `from_pb` decode unit test** — assert the new fields decode verbatim from a fixture `ExascriptInfo`, including present vs absent optionals (`crates/exa-zmq-protocol/src/meta.rs` test module).
8. **Add an end-to-end integration scenario** — a fixture UDF that emits one or more handshake values (e.g. `session_id`, `node_id`, `script_name`) so the DB roundtrip confirms live non-neutral values reach UDF code (`crates/it/tests/db_roundtrip.rs` plus a fixture UDF crate alongside the existing fixtures). **Also wire the new fixture crate into CI:** add `-p <crate>` to the "Build UDF .so artifacts (release)" step in `.github/workflows/ci.yml` (CI builds fixtures from an explicit allowlist, not `default-members`; the IT matrix downloads the uploaded `target/release/lib*.so`). Omitting this passes locally but fails the CI IT matrix with `reading UDF artifact .../lib<name>.so: No such file or directory`. *(Added post-record 2026-07-01 — this sub-step was missed in the original plan and caused the PR #40 CI failure.)*
9. **Bump `[workspace.package].version` and the pinned `exasol-udf-sdk` entry, regenerate `Cargo.lock`** — per CLAUDE.md SemVer rule (MINOR: additive SDK surface + dead-code removal).

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1, Task 4 (both edit `meta.rs`/`lib.rs`; coordinate but can land together) |
| Group B | Task 2 |
| Group C | Task 3 |
| Group D | Task 5, Task 6, Task 7 |
| Group E | Task 8, Task 9 |

Sequential dependencies:
- Group A (UdfMeta fields) → Group B/C (accessors + bridge read those fields)
- Group B + Group C → Group D (tests exercise the accessors and bridge)
- Group C → Group E (integration test needs the bridge override)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Field | `UdfMeta::conn_info` (`crates/exa-zmq-protocol/src/meta.rs`) | Write-only; read nowhere in production. Pre-ADR-018 leftover |
| Handshake-loop branch | `crates/exa-udf-runtime/src/lib.rs` (`conn_info` buffering lines) | Buffered into the now-removed field; on-demand `MT_IMPORT` is the live path |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| UdfContext exposes handshake identity and origin metadata | Unit | `crates/exasol-udf-sdk/src/context.rs` | `default_handshake_metadata_is_neutral` |
| UdfContext exposes the per-instance memory limit (regression) | Unit | `crates/exasol-udf-sdk/src/context.rs` | `default_memory_limit_is_zero` |
| Bridge surfaces handshake identity and origin metadata to the UDF | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `bridge_returns_handshake_metadata` |
| Handshake metadata carries no buffered connect-back credentials | Unit | `crates/exa-zmq-protocol/src/meta.rs` | `from_pb_decodes_handshake_metadata_without_conn_info` |
| End-to-end: live handshake values reach UDF code | Integration | `crates/it/tests/db_roundtrip.rs` | `handshake_metadata_udf_emits_session_and_node` |

Note: the SDK-default and bridge-override accessors are pure computation over in-memory state (no I/O), so they are covered by unit tests; the live-value path is covered by the integration scenario.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-sdk + runtime/dispatch-run-loop | Build the handshake-metadata fixture UDF, upload, and run `SELECT handshake_meta()` against a local Exasol Docker DB | Returns a non-zero `session_id` / valid `node_id` and the registered script name |
| protocol/wire-protocol | `cargo test -p exa-zmq-protocol` | `UdfMeta` compiles without `conn_info`; from_pb test passes |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Unit test | `cargo test` | 0 failures |
| Integration test | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
