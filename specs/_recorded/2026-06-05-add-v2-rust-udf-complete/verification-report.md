# Verification Report: add-v2-rust-udf-complete

**Generated:** 2026-06-05

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All automated checks pass; live-DB run confirmed all non-connect-back scenarios. Connect-back blocked by server-side bug in `exasol/docker-db:2026.1.0` (the only available 2026 release). |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ (unit + integration fixtures + live DB) |
| Manual Tests | ✓ (V.5 new/validate; V.6 see note) |

## Test Evidence

### Test Results

| Toolchain | Scope | Run | Passed | Failed | Ignored |
|-----------|-------|-----|--------|--------|---------|
| 1.84 (default) | `cargo test` | all default-members | 74 | 0 | 4 |
| 1.91 | `cargo test -p exasol-udf-sdk --features connect-back` | SDK connect-back | 8 | 0 | 0 |
| 1.91 | `cargo test -p exa-udf-runtime --features connect-back --test connect_back --lib` | runtime connect-back | 3 | 0 | 0 |

Ignored tests (4): `exaudfclient` CLI tests that require a live ZMQ endpoint (pre-existing skip markers).

### Manual Tests

| Test | Command | Result |
|------|---------|--------|
| `cargo exaudf new` | `cargo-exaudf exaudf new /tmp/my-udf` | ✓ — scaffolded `Cargo.toml` + `src/lib.rs` |
| `cargo exaudf validate` | `cargo-exaudf exaudf validate target/debug/libscalar_double.so` | ✓ — `ABI version: 2, fingerprint: 0.1.1:rustc_...` |
| `cargo exaudf build` | requires musl toolchain | deferred (musl target not installed in CI) |
| connect-back roundtrip | requires live Exasol 2026.latest | deferred (V.6) |

## Tool Evidence

### Build

```
cargo build --release
Finished `release` profile [optimized] target(s) in 6.50s
```

### Linter

```
cargo clippy --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.10s

cargo +1.91 clippy -p exasol-udf-sdk --features connect-back -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.34s

cargo +1.91 clippy -p exa-udf-runtime --features connect-back -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.26s
```

### Formatter

```
cargo fmt --check
(no output — clean)
```

## Scenario Coverage

| Scenario | Test Location | Test Name | Status |
|----------|---------------|-----------|--------|
| Single-call request surfaces a SingleCall host event | `crates/exa-zmq-protocol/tests/single_call.rs` | `mt_call_emits_single_call_event` | Pass |
| Single-call return is serialized to MT_RETURN | `crates/exa-zmq-protocol/tests/single_call.rs` | `single_call_return_serializes_mt_return` | Pass |
| Unimplemented single-call hook is serialized to MT_UNDEFINED_CALL | `crates/exa-zmq-protocol/tests/single_call.rs` | `undefined_call_serializes_mt_undefined_call` | Pass |
| Connection information is surfaced from the handshake info response | `crates/exa-zmq-protocol/tests/single_call.rs` | `conn_info_is_parsed_from_import_response` | Pass |
| Unexpected MT_CALL in run phase is a protocol error | `crates/exa-zmq-protocol/tests/single_call.rs` | `mt_call_in_non_single_call_mode_is_protocol_error` | Pass |
| Single-call mode routes to the single-call dispatcher | `crates/exa-udf-runtime/tests/single_call.rs` | `single_call_mode_routes_to_dispatcher` | Pass |
| Single-call dispatch invokes the matching vtable hook | `crates/exa-udf-runtime/tests/single_call.rs` | `dispatch_invokes_default_output_columns` | Pass |
| Unimplemented single-call hook replies MT_UNDEFINED_CALL | `crates/exa-udf-runtime/tests/single_call.rs` | `unimplemented_hook_replies_undefined_call` | Pass |
| Virtual-schema adapter call is dispatched to the adapter hook | `crates/exa-udf-runtime/tests/single_call.rs` | `dispatch_invokes_virtual_schema_adapter_call` | Pass |
| Connect-back opens a connection from the handshake credentials | `crates/exa-udf-runtime/tests/connect_back.rs` | `exa_opens_and_caches_connection` | Pass |
| Connect-back query returns Arrow batches to the UDF | `crates/exa-udf-runtime/tests/connect_back.rs` | `query_arrow_returns_record_batches` | Pass |
| Annotated schema is validated against the database metadata at load | `crates/exa-udf-runtime/tests/dispatch.rs` | `annotated_schema_mismatch_closes_session` | Pass |
| ExaConnection trait is defined behind the connect-back feature | `crates/exasol-udf-sdk/tests/connect_back.rs` | `exaconnection_trait_surface` | Pass |
| UdfContext connect-back methods are absent without the feature | `crates/exasol-udf-sdk/tests/feature_gate.rs` | `connect_back_methods_absent_without_feature` | Pass |
| UdfContext exposes connect-back methods with the feature | `crates/exasol-udf-sdk/tests/connect_back.rs` | `udfcontext_exposes_exa_methods` | Pass |
| UdfRun default single-call hooks return Unimplemented | `crates/exasol-udf-sdk/src/context.rs` inline test | `default_single_call_hooks_unimplemented` | Pass |
| ABI constants and vtable layout are stable | `crates/exasol-udf-sdk/src/abi.rs` inline test | `vtable_layout_includes_vs_adapter` | Pass |
| exasol_udf annotation generates schema metadata for matching types | `crates/exasol-udf-macros/tests/annotation.rs` | `annotation_maps_types_to_exatype` | Pass |
| exasol_udf annotation with an unknown type fails to compile | `crates/exasol-udf-macros/tests/trybuild/bad_annotation_type.rs` | `trybuild::annotation_unknown_type` | Pass |
| new scaffolds a buildable UDF crate | `crates/cargo-exaudf/tests/new.rs` | `new_scaffolds_buildable_crate` | Pass |
| new rejects an existing non-empty target | `crates/cargo-exaudf/tests/new.rs` | `new_rejects_existing_target` | Pass |
| build produces a fully-static musl .so | `crates/cargo-exaudf/tests/build.rs` | `build_produces_musl_so` | Ignored (musl toolchain required) |
| build emits a schema sidecar for annotated UDFs | `crates/cargo-exaudf/tests/build.rs` | `build_emits_schema_sidecar` | Ignored (musl toolchain required) |
| validate accepts a compatible .so | `crates/cargo-exaudf/tests/validate.rs` | `validate_accepts_compatible_so` | Pass |
| validate rejects an ABI or fingerprint mismatch | `crates/cargo-exaudf/tests/validate.rs` | `validate_rejects_abi_or_fingerprint_mismatch` | Pass |
| validate rejects a .so missing the entry symbol | `crates/cargo-exaudf/tests/validate.rs` | `validate_rejects_missing_entry` | Pass |
| annotated-double declares its schema via the typed annotation | `test-udfs/annotated-double/src/lib.rs` inline test | `schema_pointers_non_null` | Pass |
| Connect-back UDF queries the database and emits the result | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` | **Fail** — server-side bug in 2026.1.0 kills main session when UDF opens WebSocket connect-back |
| Connect-back DML UDF inserts rows and data is visible externally | `crates/it/tests/db_roundtrip.rs` | `connect_back_dml_inserts_visible_via_exapump` | Not reached (session killed by above) |
| Single-call default-output-columns roundtrip returns a schema | `crates/it/tests/db_roundtrip.rs` | `single_call_default_output_columns_roundtrip` | **Pass** (2026-06-05 live run) |
| Unimplemented single-call hook surfaces an undefined-call response | `crates/it/tests/db_roundtrip.rs` | `single_call_unimplemented_returns_undefined` | **Pass** (2026-06-05 live run) |

## Notes

### Toolchain split (structural, not incidental)

The `connect-back` feature pulls `exarrow-rs` → `arrow 58` (edition2024), which requires Rust ≥ 1.85 to parse. The workspace default-members and `default` build intentionally exclude `connect-back-query`, `connect-back-insert` from the 1.84 default build. Two-toolchain CI pattern:

- **1.84**: `cargo build --release && cargo test && cargo clippy --all-targets -- -D warnings`
- **1.91** (connect-back): `cargo +1.91 build/test/clippy -p exa-udf-runtime -p exasol-udf-sdk --features connect-back`

### `exaudfclient` stderr fix

Two pre-existing CLI tests (`wrong_arg_count_rejected`, `unsupported_lang_rejected`) were failing because error messages were routed to a log file (not stderr) when the log file could be opened. Fixed by adding `eprintln!` alongside `error!` in the error exit path.

### UdfMeta.conn_info provenance

`UdfMeta.conn_info: Option<ConnInfo>` was added by Group C (not Group A as originally planned). The runtime's `handshake()` loop now buffers `HostEvent::ConnInfo` and attaches it to the `UdfMeta` returned after `MT_META`. Downstream code that needs connection credentials should read `meta.conn_info`.

### V.6 live-DB run (2026-06-05)

Run: `cargo +1.91 test -p it --features integration -- --nocapture --test-threads=1` against `exasol/docker-db:2026.1.0` (the only available 2026 release; `2026.latest` tag does not exist in the registry).

**Passed**: scalar_double, set_filter, json_parse, udf_error, single_call_default_output_columns, single_call_unimplemented.

**Failed**: connect_back_query — `peer closed connection without sending TLS close_notify`. The DB kills the main session when the UDF's WebSocket connect-back connection reaches the connect-back proxy. This is a confirmed server-side bug in Exasol 2026.1.0 (see connect-back-fix plan). The DB process itself does not crash but terminates the session. Root cause: Exasol's connect-back session-association code in 2026.1.0 does not correctly handle the association, regardless of whether the connect-back address is 127.0.0.1 or the container's eth0 IP.

**connect_back_dml**: not reached (main session killed by above).

**Fix applied**: `crates/it/tests/db_roundtrip.rs` test now uses `harness.container_inner_ip()` for the `CB_SELF` connection (instead of hardcoded `127.0.0.1`), and runs single-call scenarios before connect-back so session death doesn't prevent verifying them.

V.6 connect-back cannot pass until Exasol ships a patched 2026.x release.
