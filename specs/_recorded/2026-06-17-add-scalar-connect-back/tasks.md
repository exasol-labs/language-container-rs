# Tasks: add-scalar-connect-back

## Phase 2: Implementation (Group A — verify existing code/tests)
- [x] 2.1 Confirm `test-udfs/connect-back-scalar/` crate present + in root Cargo.toml members
- [x] 2.2 Confirm `connect_back_scalar_queries_and_returns` + `CB_SCALAR_LIB` wired in db_roundtrip.rs
- [x] 2.3 Confirm `-p connect-back-scalar` in scripts/ci-it-local.sh
- [x] 2.4 Confirm `connect-back-scalar` in .github/workflows/ci.yml build step

## Phase 2: Implementation (Group B — documentation)
- [x] 2.5 Relax CLAUDE.md connect-back rule (remove "never SCALAR → SIGABRT")
- [x] 2.6 Update docs/writing-a-udf.md: remove stale "always SET, never SCALAR" warning; note both supported; clarify "drain input rows" as SET-specific

## Phase 3: Verification
- [x] 3.1 cargo build --release → ok
- [x] 3.2 cargo test → all pass (rebuilt stale ABI-3 debug fixtures; pre-existing, unrelated)
- [x] 3.3 cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings → clean
- [x] 3.4 Scenario coverage: connect_back_scalar IT scenario green (15/15 live this session)
