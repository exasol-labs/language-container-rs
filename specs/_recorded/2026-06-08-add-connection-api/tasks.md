# Tasks: add-connection-api

## Group A: SDK types + trait (parallel)
- [x] 1.1 Add `ConnectionObject` struct to `connect_back.rs`; remove `ConnectBackOptions`
- [x] 1.2 Update `lib.rs` re-exports: export `ConnectionObject, ExaConnection`; drop `ConnectBackOptions`
- [x] 2.1 Replace `exa`/`exa_named`/`exa_connect` in `context.rs` with `cluster_ip`, `connection`, `connect_back` methods

## Group B: Endpoint plumbing (after Group A)
- [x] 3.1 Add `parse_cluster_ip` helper in `artifact.rs` [expert]
- [x] 3.2 Thread ZMQ endpoint from `Runtime` → `dispatch::run_udf` → `run_batch` → `HostContextBridge`

## Group C: Bridge rewiring (after A + B)
- [x] 4.1 Rewire `HostContextBridge`: replace `exa()` with `cluster_ip()`, `connection(name)`, `connect_back(&obj)` [expert]
- [x] 4.2 Generalise `conn_requester` to take connection name; map `ConnInfo` ↔ `ConnectionObject`
- [x] 5.1 Remove proactive `conn_info` handshake seeding from `dispatch.rs`/`lib.rs`

## Group D: Tests + docs (after Group C)
- [x] 6.1 Rewrite `crates/exasol-udf-sdk/tests/connect_back.rs` for new API
- [x] 6.2 Update `crates/exasol-udf-sdk/tests/feature_gate.rs` for new absent method names
- [x] 7.1 Rewrite `crates/exa-udf-runtime/tests/connect_back.rs` mock tests for new API
- [x] 7.2 Add `parse_cluster_ip` unit test in `artifact.rs`
- [x] 7.3 Add integration test `connection_fetches_credentials_via_mt_import` in runtime tests
- [x] 8.1 Update example UDFs and SDK rustdoc that referenced `exa()`/`ConnectBackOptions`

## Group E: Test UDFs + integration harness (after Group C)
- [x] 9.1 Rewrite `test-udfs/connect-back-query/src/lib.rs` to new API [expert]
- [x] 9.2 Rewrite `test-udfs/connect-back-insert/src/lib.rs` to new API [expert]
- [x] 9.3 Add `test-udfs/connect-back-cluster-ip/` scalar UDF
- [x] 10.1 Add `connect_back_cluster_ip_emits_node_ip` scenario to integration harness [expert]
- [x] 10.2 Update existing KNOWN_FAILING integration scenarios to use new API [expert]
- [x] 10.3 Add `integration/db-roundtrip` spec delta

## Verification
- [x] V.1 Run `cargo test -p exasol-udf-sdk --features connect-back`
- [x] V.2 Run `cargo +1.91 test -p exa-udf-runtime --features connect-back`
- [x] V.3 Run `cargo +1.91 build --release`
- [x] V.4 Run `cargo +1.91 clippy --all-targets --all-features -- -D warnings`
- [x] V.5 Run `cargo fmt --check`
- [x] V.6 Run `cargo +1.91 test -p it --features integration -- --nocapture`
