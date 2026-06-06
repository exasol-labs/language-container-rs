# Tasks: fix-connect-back-external-client

## Phase 1: Runtime transport hardening (Group A)
- [x] 1.1 Remove `features = ["websocket"]` from the `exarrow-rs` dependency in `crates/exa-udf-runtime/Cargo.toml`
- [x] 1.2 Update `open_connection` and `build_dsn` doc comments in `crates/exa-udf-runtime/src/connect_back.rs` to state new-session/new-transaction external-client semantics

## Phase 2: Integration harness alignment (Group A)
- [x] 2.1 Change `DB_TAG` in `crates/it/src/lib.rs` from `2026.1.0` to `2026.latest`
- [x] 2.2 Update `container_connect_back_address` doc comment in `lib.rs` and connect-back comments in `db_roundtrip.rs` to record 2026-06-06 re-verification; replace "decision [15]" with ADR-015 reference

## Phase 3: Documentation (Group A)
- [x] 3.1 Create `README.md` with connect-back section
- [x] 3.2 Add ADR-014 and ADR-015 to `specs/decision-log.md`

## Phase 4: Known-failing gate (Group B)
- [x] 2.3 Keep connect-back scenarios in `db_roundtrip.rs` as known-failing: replace `cb_query_result?` / `cb_dml_result?` with graceful known-failing handling that still dumps diagnostics and does not assert a false pass [expert]

## Phase 5: Verification (Group C)
- [x] 4.1 Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo build --release`
- [x] 4.2 Re-run integration suite: confirm 6/8 pass, connect-back scenarios fail with SIGABRT signature [expert]
