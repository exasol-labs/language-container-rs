# Tasks: drop-unused-deps-and-dead-code

## Group A — Manifest/file edits (independent)
- [x] 1.1 Remove `anyhow` from `crates/exa-udf-runtime/Cargo.toml`
- [x] 1.2 Remove `anyhow` from `crates/exaudfclient/Cargo.toml`
- [x] 1.3 Remove `prost-types` from `crates/exa-proto/Cargo.toml`
- [x] 1.4 Remove `indexmap` from root `Cargo.toml` `[workspace.dependencies]`
- [x] 1.5 Remove `arrow` from `test-udfs/connect-back-query/Cargo.toml` + delete stale doc comment
- [x] 4.1 Remove getrandom probe `eprintln!` and `libc` dep from `crates/exaudfclient`
- [x] 4.2 Remove `udf_diag.log` file-logging path from `crates/exaudfclient/src/main.rs`
- [x] 4.3 Delete `Dockerfile.debian`
- [x] 4.4 Apply `container/slim-image` spec delta: remove stale musl scenario, fix 1.91→1.92

## Group B — Dead test fixture removals
- [x] 2.1 Remove `test-udfs/spike-connect/` crate and drop from root `Cargo.toml` members/default-members
- [x] 2.2 Fold Decimal annotation into macro test; remove `test-udfs/annotated-double/` crate

## Group C — Internal dead pub shrink
- [x] 3.1 Remove 10 never-constructed `HostAction` variants from `crates/exa-zmq-protocol/src/messages.rs` [expert]
- [x] 3.2 Shrink unused `pub` surface in `crates/exa-zmq-protocol` [expert]
- [x] 3.3 Narrow `LoadedUdf::annotated_input_schema` and `annotated_output_schema` to `pub(crate)`

## Group D — Version bump (runs LAST)
- [x] 5.1 Bump workspace version to 0.15.1 and update SDK pin in root `Cargo.toml`
- [x] 5.2 Regenerate `Cargo.lock` via `cargo build` and confirm removed deps

## Phase 4: Code Review
- [x] CR.1 Code review of all changed files — no material defects

## Phase 5: Verification
- [x] V.1 Build: `cargo build --release`
- [x] V.2 Clean rebuild exa-proto: `cargo clean -p exa-proto && cargo build -p exa-proto`
- [x] V.3 Tests: `cargo test` (dispatch fixture .so rebuilt at 0.15.1 first)
- [x] V.4 Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- [x] V.5 Format: `cargo fmt --check`
- [x] V.6 Manual checks from plan

## Phase 6: Verification Report
- [x] R.1 Generate verification-report.md
