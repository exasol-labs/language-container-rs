# Tasks: add-udfcontext-handshake-metadata

## Phase 2: Implementation (Group A — sequential, shared files)
- [x] 2.1 Extend `UdfMeta` with undecoded handshake fields + map in `from_pb` (`crates/exa-zmq-protocol/src/meta.rs`) [expert]
- [x] 2.2 Add defaulted `UdfContext` accessors (`crates/exasol-udf-sdk/src/context.rs`) [expert]
- [x] 2.3 Thread fields into `HostContextBridge` + override accessors; update construction sites (`crates/exa-udf-runtime/src/rowset.rs`, `dispatch.rs`) [expert]
- [x] 2.4 Remove dead `conn_info` handshake buffering; keep `ConnInfo`/`HostEvent::ConnInfo` (`meta.rs`, `crates/exa-udf-runtime/src/lib.rs`) [expert]
- [x] 2.5 Add SDK default-value unit tests (`crates/exasol-udf-sdk/src/context.rs`)
- [x] 2.6 Add bridge-override unit tests (`crates/exa-udf-runtime/src/rowset.rs`)
- [x] 2.7 Add `from_pb` decode unit test (`crates/exa-zmq-protocol/src/meta.rs`)
- [x] 2.8 Add E2E integration scenario + fixture UDF (`crates/it/tests/db_roundtrip.rs`, new fixture crate) [expert]
- [x] 2.9 Bump workspace version (0.19.1 → 0.20.0) + pinned `exasol-udf-sdk` entry + regenerate Cargo.lock

## Phase 4: Code Review
- [x] 4.1 Review all changed files (PASS on 4 correctness props; minor findings noted)

## Phase 5: Verification
- [x] 5.1 Build (cargo build --release)
- [x] 5.2 Unit test suite (cargo test) — 0 failures
- [x] 5.3 Lint (cargo clippy --all-targets --all-features -- -D warnings) — clean
- [x] 5.4 Format (cargo fmt --check) — clean
- [x] 5.5 Integration tests (cargo test -p it --features integration) — 0 failures
- [x] 5.6 E2E tests — green

## Phase 6: Post-record fix (CI wiring — missed in original plan)
- [x] 6.1 Wire the new `handshake-meta` fixture into the CI "Build UDF .so artifacts (release)" `-p` allowlist (`.github/workflows/ci.yml`). Missed by Task 2.8, which added the fixture crate + IT scenario but not the CI build entry, so IT passed locally (`.so` built via `default-members`) yet failed on the CI matrix with `reading UDF artifact .../libhandshake_meta.so: No such file or directory`.
- [x] 6.2 Add a CLAUDE.md guardrail: adding a `test-udfs/*` fixture requires adding it to the CI `-p` allowlist (CI does not use `default-members`).
