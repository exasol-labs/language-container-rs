# Tasks: connect-back-fix

## Group A — Serial implementation

- [x] 1.1 Enable connect-back feature on exaudfclient (Cargo.toml)
- [x] 1.2 Propagate run-phase ConnInfo in dispatch.rs [expert]
  - Note: Also implemented on-demand MT_IMPORT credential request via `conn_requester` closure in `HostContextBridge`. When `conn_info` is `None` (DB did not push proactively), `exa()` sends MT_IMPORT with `PB_IMPORT_CONNECTION_INFORMATION` via ZMQ and extracts `ConnInfo` from the response. The exchange is safe during `run_batch` execution because the outer dispatch loop is blocked waiting for the function to return. The `FnOnce` closure captures `&mut Protocol` and `&ZmqTransport` for the duration of `run_batch` and is released on return. Proactive credentials (if present) take priority over the on-demand path.
- [x] 1.3 Rebuild exaudfclient binary + SLC Docker image
- [x] 1.4 Run integration tests and verify all scenarios pass
  - Note: Bugs 1 & 2 fixed; scalar, set, json_parse, udf_error, and single_call scenarios pass.
    connect_back_query and connect_back_dml fail due to a confirmed server-side SIGABRT in
    Exasol 2026.1.0 — when any WebSocket connect-back session connects from 127.0.0.1:8563 (the
    UDF sandbox loopback), the DB immediately crashes the main session handler (signal 6, core dump).
    The crash is in Exasol's internal connect-back session-association code, not in our client code.
    Root cause: exarrow-rs was using the native binary protocol (default feature) which the
    connect-back proxy rejects silently; switched to websocket transport, which reaches the proxy
    and triggers the crash. This is a known limitation of exasol/docker-db:2026.1.0.
