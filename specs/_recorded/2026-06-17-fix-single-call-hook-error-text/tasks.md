# Tasks: fix-single-call-hook-error-text

## Phase 1: Branch
- [x] 1.1 Create feature branch `feat/fix-single-call-hook-error-text` off `main`

## Phase 2: Implementation (Group A — parallel)
- [x] 2.1 Fix `call_noarg_hook`, `call_arg_hook`, `call_ctx_arg_hook` in `crates/exa-udf-runtime/src/loader.rs` to surface error text; add `#[cfg(test)]` unit tests covering error-text surfaced, null fallback, and success path
- [x] 2.2 Bump workspace `version = "0.11.0"` → `"0.11.1"` in `Cargo.toml`

## Phase 3: Verification
- [x] 3.1 Build: `cargo build --release -p exa-udf-runtime`
- [x] 3.2 Test: `cargo test -p exa-udf-runtime`
- [x] 3.3 Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 3.4 Format: `cargo fmt --check`

## Phase 4: PR
- [ ] 4.1 Open PR against `main` with `fix:` Conventional Commit title referencing issue #11
