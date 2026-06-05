# Verification Report: add-v1-rust-udf-slim

**Generated:** 2026-06-05

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All six verification checks pass. All four DB round-trip scenarios pass against a live Exasol 2026.1.0 container. Two bugs fixed during this session: ZMQ framing (REQ→DEALER+empty-frame) and BIGINT type mapping (Int64→Numeric). |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Build musl UDFs | ✓ |
| Build image | ✓ |
| Tests (unit + lib) | ✓ |
| Tests (integration, real DB) | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Bugs Fixed During This Session

### Bug 1 — ZMQ socket type: REQ instead of DEALER

**Root cause:** `transport.rs` used `zmq::REQ`. The Exasol DB binds a `zmq::ROUTER` socket. The REQ socket auto-injects an empty delimiter frame, making the ROUTER see `[req_id, empty, payload]` where it expects `[dealer_id, empty, payload]` with the client explicitly supplying the empty delimiter. After MT_CLIENT was sent, the DB never replied with MT_INFO because the envelope was garbled.

**Fix:** Changed socket type to `zmq::DEALER`. DEALER sends `[empty][payload]` explicitly (via `SNDMORE`) and receives `[empty][payload]` back; the ROUTER strips/restores the identity on both sides. The transport test mock server was updated from `zmq::REP` to `zmq::ROUTER` with proper three-frame envelope handling.

**Files changed:**
- `crates/exa-zmq-protocol/src/transport.rs`
- `crates/exa-zmq-protocol/tests/transport.rs`

### Bug 2 — BIGINT column type: Value::Int64 instead of Value::Numeric

**Root cause:** Exasol maps `BIGINT` to `DECIMAL(36,0)`, which the ZMQ protocol sends as `PB_NUMERIC` (column type 4) with values in `data_string`. Both `scalar_double` and `set_filter` matched only `Value::Int64`, which is only produced for `DECIMAL(8,0)` (`PB_INT64`). The `match ctx.get(0)?` arm fell through to the error case, returning exit code 1.

**Fix:** Added a `Value::Numeric(s)` arm to both UDFs: parse the decimal string as `i64`, perform the computation, and re-emit as `Value::Numeric(result.to_string())`. This keeps the value in the `data_string` block that the DB expects for PB_NUMERIC output columns.

**Files changed:**
- `test-udfs/scalar-double/src/lib.rs`
- `test-udfs/set-filter/src/lib.rs`

## Test Evidence

### Test Results

| Suite | Run | Passed | Failed |
|-------|-----|--------|--------|
| `exa-zmq-protocol` unit | 13 | 13 | 0 |
| `exa-zmq-protocol` integration (transport) | 2 | 2 | 0 |
| `it` unit (`decode_base64`) | 1 | 1 | 0 |
| `it` integration (DB round-trip) | 1 | 1 | 0 |
| All workspace (unit + lib) | See notes | all pass | 0 |

### Integration Test Output (abridged)

```
[it] starting exasol container
[it] container up; connecting
[it] connected
[it] SELECT 1 ok
[it] exporting + uploading SLC to BucketFS
[it] SLC uploaded; registering SCRIPT_LANGUAGES
[it] SLC registered; uploading UDF artifacts
[it] scenario scalar_double ok
[it] scenario set_filter ok
[it] scenario json_parse ok
[it] scenario udf_error ok
test db_roundtrip_all_scenarios ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 48.87s
```

### Manual Tests

| Feature | Command | Result |
|---------|---------|--------|
| workspace-bootstrap | `cargo build --release` | ✓ exit 0 |
| wire-protocol | `cargo test -p exa-zmq-protocol` | ✓ 15 tests pass |
| musl UDFs | `cargo build --release --target targets/x86_64-unknown-linux-musl-dylib.json -p scalar-double -p set-filter -p json-parse -Z build-std=std,panic_abort` | ✓ three `.so` produced |
| slim-image | `docker build -t slc-rs-slim:dev --build-context exarrow-rs=/home/talos/code/exarrow-rs .` | ✓ image built |
| db-roundtrip | `cargo +1.91 test -p it --features integration -- --nocapture` | ✓ all 4 scenarios pass |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 05s
```

Zero errors or warnings.

### Formatter

```
cargo fmt --check
(no output — all files formatted correctly)
```

## Scenario Coverage

All plan-specified DB round-trip scenarios pass against a live `exasol/docker-db:2026.1.0` container:

| Scenario | Test Name | Result |
|----------|-----------|--------|
| scalar `double_it(21)` → 42 | `db_roundtrip_all_scenarios` → `scalar_double_returns_42` | ✓ Pass |
| set/EMITS filter_positive → positives only | `db_roundtrip_all_scenarios` → `set_filter_emits_positive_only` | ✓ Pass |
| `json_field('{"name":"exa"}')` → `exa` | `db_roundtrip_all_scenarios` → `json_parse_extracts_name` | ✓ Pass |
| UDF error surfaces `F-UDF-CL-RUST-` | `db_roundtrip_all_scenarios` → `udf_error_surfaces_prefix` | ✓ Pass |
| ZMQ transport connects to ROUTER | `transport_connects_to_ipc` | ✓ Pass |
| ZMQ transport round-trip single frame | `transport_round_trip_single_frame` | ✓ Pass |
| Handshake MT_INFO→MT_META | `handshake_emits_info_then_meta` | ✓ Pass |
| Scalar loop next/emit/done | `scalar_loop_next_emit_done` | ✓ Pass |
| Set loop multiple batches | `set_loop_multiple_batches` | ✓ Pass |
| Close sequence | `close_sequence_cleanup_finished_close` | ✓ Pass |
| Ping-pong echoed | `ping_pong_echoes` | ✓ Pass |
| Reset restarts iteration | `reset_restarts_iteration` | ✓ Pass |
| TryAgain no phase advance | `try_again_no_phase_advance` | ✓ Pass |
| Unexpected message is error | `unexpected_message_is_error` | ✓ Pass |
| Error close path prefix | `error_close_path_prefix` | ✓ Pass |

## Notes

- The workspace is pinned to Rust 1.84 but the `it` crate has `rust-version = "1.85"` due to a transitive dependency (`getrandom v0.4.2`) requiring `edition2024`. The integration test is run with the separately installed 1.91 toolchain: `cargo +1.91 test -p it --features integration`.
- Musl dylib artifacts use a custom target spec (`targets/x86_64-unknown-linux-musl-dylib.json`) and require `RUSTC_BOOTSTRAP=1 -Z build-std=std,panic_abort` because Rust 1.84's built-in musl target has `dynamic-linking=false`.
- The Docker image must be built with `--build-context exarrow-rs=/home/talos/code/exarrow-rs` because the workspace patches `exarrow-rs` from a local path.
