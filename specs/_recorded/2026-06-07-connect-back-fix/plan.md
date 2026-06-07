# Plan: connect-back-fix

## Problem

Two bugs prevent the `connect_back_udf_queries_and_emits` integration test from passing:

### Bug 1 — Missing `connect-back` feature on `exaudfclient`

`exaudfclient/Cargo.toml` does NOT enable the `connect-back` feature on `exa-udf-runtime`.

When `connect-back` is absent, the `UdfContext` trait (in `exasol-udf-sdk`) has three methods fewer:
`exa()`, `exa_named()`, `exa_connect()`. The `dyn UdfContext` fat-pointer vtable therefore has
a different layout than what the UDF `.so` expects (which was compiled WITH `connect-back`).
When the UDF calls `ctx.exa()`, it dispatches through a vtable slot that doesn't exist in
the runtime's layout — undefined behaviour, process crash, Exasol reports "VM crashed".

### Bug 2 — Run-phase `MT_IMPORT` (`ConnInfo`) is silently dropped

The Exasol DB sends `MT_IMPORT` (connection credentials) as a reply to the client's first
`MT_NEXT` request (not during the handshake). The current `consume_input` match arm

```rust
HostEvent::ConnInfo(_) => {}
```

discards the credentials. `HostContextBridge` is therefore built with `conn_info = None`,
so `exa()` (even after fix 1) would return `Err(UdfError::ConnectBack("no connection
information in handshake"))`.

## Fix Design

### Fix 1 — Enable `connect-back` on `exaudfclient`

File: `crates/exaudfclient/Cargo.toml`

Change:
```toml
exa-udf-runtime = { path = "../exa-udf-runtime" }
```
To:
```toml
exa-udf-runtime = { path = "../exa-udf-runtime", features = ["connect-back"] }
```

### Fix 2 — Capture run-phase `ConnInfo` in `consume_input` / `run_udf`

Files: `crates/exa-udf-runtime/src/dispatch.rs`

Strategy: propagate `ConnInfo` from the run phase into `run_batch`.

- Change `run_udf` to hold a local `#[cfg(feature = "connect-back")] let mut run_conn_info: Option<ConnInfo> = meta.conn_info.clone();` that starts from the handshake value and gets updated by `consume_input` when `ConnInfo` arrives.
- Change `consume_input` to accept `#[cfg(feature = "connect-back")] conn_info: &mut Option<ConnInfo>` and update it when `HostEvent::ConnInfo(ci)` is received.
- Change `run_batch` to accept `#[cfg(feature = "connect-back")] conn_info: Option<ConnInfo>` instead of always reading `meta.conn_info`.

This keeps `UdfMeta` immutable and avoids interior-mutability complexity.

## Parallelization

Group A (serial, blocking each other):
- Task 1: Fix 1 — feature flag
- Task 2: Fix 2 — run-phase ConnInfo propagation [expert]
- Task 3: Rebuild exaudfclient + SLC image
- Task 4: Verification — run integration tests

## Verification

### Checklist
- `cargo build -p exaudfclient` exits 0
- `cargo test -p exa-udf-runtime` exits 0 (all unit tests pass)
- `cargo +1.91 test -p it --features integration -- --nocapture` exits 0 (full integration suite)

### Scenario Coverage
- `connect_back_udf_queries_and_emits` passes (returns 42)
- `connect_back_dml_inserts_visible_via_exapump` passes

### Manual Testing
N/A — covered by integration tests.
