# Tasks: fix-abi-feature-safety

## Phase 2: Implementation (Group A — SDK) [expert]
- [x] 2.1 Remove `query_arrow` from `ExaConnection`; make `query_for_each` required; keep `query`/`execute`/txn defaults (connect_back.rs) — #26
- [x] 2.2 Un-gate `connect_back` module + `ConnectionObject`/`ExaConnection` re-exports (lib.rs); remove SDK `connect-back` cargo feature (Cargo.toml) — #26
- [x] 2.3 Update SDK + runtime mock tests to implement `query_for_each` not `query_arrow`; drop `query_arrow` asserts
- [x] 2.4 Remove `#[cfg(feature=...)]` from `UdfContext` method decls (cluster_ip/connection/connect_back/emit_record_batch_ipc); always-declared w/ `Unimplemented` defaults (context.rs) [expert] — #31
- [x] 2.5 Narrow SDK `emit-arrow` to gate only `dep:arrow` + `EmitBatch` ext-trait; `emit_record_batch_ipc(&[u8])` always declared [expert] — #31
- [x] 2.6 Bump `EXA_UDF_ABI_VERSION` 4 → 5 (abi.rs); leave `ExaUdfVTable` field order unchanged [expert]

## Phase 2: Implementation (Group B — runtime + manifests, after A)
- [x] 2.7 Update `exa-udf-runtime/Cargo.toml`: `connect-back` feature must not reference removed `exasol-udf-sdk/connect-back`; keep gating exarrow-rs/tokio/rustls + `RuntimeExaConnection` (done by Group A agent)
- [x] 2.8 Implement `query_for_each` directly in runtime/connect_back.rs (stream → record_batch_to_rows → callback → drop); remove `query_arrow` impl
- [x] 2.9 Audit `test-udfs/*/Cargo.toml` for `exasol-udf-sdk/connect-back` refs and drop them (done by Group A agent)

## Phase 2: Tests (after A+B)
- [x] 2.10 Add/adjust verification tests per plan Scenario Coverage

## Phase 3: Code Review
- [x] 3.1 Review all changed files (2 PR-added-test findings fixed; pre-existing /tmp cb_log + fetch_all streaming noted as out-of-scope follow-ups)

## Phase 4: Verification
- [x] 4.1 Build (default + all-features) — exit 0
- [x] 4.2 cargo test — 159 passed, 0 failed, 4 ignored (rebuilt stale debug fixtures for v0.18.0 fingerprint)
- [x] 4.3 Lint + format — clippy clean, fmt clean
