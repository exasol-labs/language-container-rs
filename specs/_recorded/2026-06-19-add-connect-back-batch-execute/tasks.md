# Tasks: add-connect-back-batch-execute

## Group A — Dependency and version prep

- [x] A.1 Bump `exarrow-rs` in `[workspace.dependencies]` from `"0.12.7"` to `"^0.12.8"` in `Cargo.toml`
- [x] A.2 Bump `[workspace.package].version` from `"0.12.1"` to `"0.13.0"` and update the `exasol-udf-sdk` workspace-dep `version` field to `"0.13.0"` in `Cargo.toml`
- [x] A.3 Run `cargo check` to regenerate `Cargo.lock` with the new versions

## Group B — SDK trait surface

- [x] B.1 Add `execute_batch` default method to `ExaConnection` trait in `crates/exasol-udf-sdk/src/connect_back.rs`
- [x] B.2 Add unit test `execute_batch_default_returns_unimplemented` in `crates/exasol-udf-sdk/tests/connect_back.rs`
- [x] B.3 Run `cargo test -p exasol-udf-sdk --features connect-back` — 9 passed, 0 failed

## Group C — Runtime implementation [expert]

- [x] C.1 Add `value_to_parameter` helper in `crates/exa-udf-runtime/src/connect_back.rs`
- [x] C.2 Implement `execute_batch` on `RuntimeExaConnection` in `crates/exa-udf-runtime/src/connect_back.rs`
- [x] C.3 Add unit test `execute_batch_value_mapping_roundtrip` in `crates/exa-udf-runtime/src/connect_back.rs`
- [x] C.4 Run `cargo test -p exa-udf-runtime --features connect-back` — 21 unit + 5 connect-back + dispatch tests all pass

## Group D — Verification and release

- [x] D.1 Run full workspace test suite: `cargo test` — 0 failures
- [x] D.2 Run `cargo clippy --all-targets --all-features -- -D warnings` — 0 warnings; `cargo fmt --check` — clean
- [x] D.3 Run integration tests + E2E via `scripts/ci-it-local.sh` — 17/17 scenarios pass
- [ ] D.4 Commit, push, create PR
