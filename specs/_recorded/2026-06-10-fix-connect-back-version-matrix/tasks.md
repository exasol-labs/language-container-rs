# Tasks: fix-connect-back-version-matrix

## Phase 3: Implementation (Group A — parallel)
- [x] 1.1 Add Cargo features `db-2025-1`, `db-2025-2`, `db-2026-1` to `crates/it/Cargo.toml`, with `default = ["db-2026-1"]`
- [x] 1.2 Add `db_series()` to `crates/it/src/lib.rs`: read `EXASOL_DB_SERIES`; fallback to compiled default feature; reject unknown values
- [x] 1.3 Update `db_tag()` to use series → default image tag mapping, keeping `EXASOL_VERSION` as explicit override
- [x] 2.1 Rewrite `cluster_ip()` in `crates/exa-udf-runtime/src/rowset.rs` using `libc::getifaddrs` [expert]
- [x] 2.2 Remove `parse_cluster_ip()` from `crates/exa-udf-runtime/src/artifact.rs` [expert]
- [x] 5.1 Remove SIGABRT-related comments from `crates/exa-udf-runtime/src/connect_back.rs` (if any)
- [x] 5.2 Grep and remove stale `SIGABRT`, `ADR-015`, `signal 6`, `Part:44`, `known-failing` references from code/comments

## Phase 3: Implementation (Group B — parallel, after Group A)
- [x] 3.1 Add `connect_back_sql_address()` method to `Harness` in `crates/it/src/lib.rs`
- [x] 3.2 Update `db_roundtrip.rs` connect-back scenarios to use `connect_back_sql_address()` instead of gateway/NAT address [expert]
- [x] 3.3 Remove `container_connect_back_address()` from `crates/it/src/lib.rs`
- [x] 4.1 Rewrite `connect_back_udf_queries_and_emits` as hard assertion (assert 42, session alive) [expert]
- [x] 4.2 Rewrite `connect_back_dml_inserts_visible_via_exapump` as hard assertion (assert 10,20,30) [expert]
- [x] 4.3 Rewrite `connect_back_cluster_ip_emits_node_ip` as hard assertion (valid IPv4) [expert]
- [x] 4.4 Remove `is_known_sigabrt_failure()`, `is_known_ipc_transport_failure()`, SIGABRT match arms, ADR-015 comment; reorder if simplified [expert]

## Phase 3: Implementation (Group C — parallel, after Group B)
- [x] 6.1 Update `build-artifacts` in `.github/workflows/ci.yml` to compile with `--features integration,db-2026-1` [expert]
- [x] 6.2 Update integration matrix: remove `--skip connect_back`, add `EXASOL_DB_SERIES` per entry [expert]
- [x] 6.3 Confirm `it-runner` invocation uses runtime env only (no per-version compile flags) [expert]
- [x] 7.1 Note in plan that ADR-013/ADR-015 are superseded (decision-log; permanent cleanup at speq record)

## Phase 5: Verification
- [x] 8.1 Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`
- [~] 8.2 Run integration suite for each series via `EXASOL_DB_SERIES` against live Docker — BLOCKED: connect_back_query/insert FAIL on all 3 series. Root cause is DB-side: Part:40 (SQL worker) SIGABRT when any external SLC opens a connect-back session (Part:44 registers). Hypothesis exhausted from client side — need coredb code to find server-side assertion/flag.
- [x] 8.3 Regression: confirm pre-existing scenarios pass unchanged

## Phase 6: Connect-back fix — change SCALAR to SET (expert)

Root cause: `connect_back_query` is declared `RUST SCALAR SCRIPT … RETURNS BIGINT`. Exasol crashes the parent SQL process (SIGABRT) when any connect-back session is established while a SCALAR UDF executes. Strata-rs Python UDFs that do connect-back are all `SET SCRIPT … EMITS (…)` and work correctly. Fix: make `connect_back_query` a SET UDF.

- [x] 9.1 Change `test-udfs/connect-back-query/src/lib.rs`: add `while ctx.next()? {}` to drain the single FROM DUAL input row; keep connect-back query + emit logic; remove unused imports. [expert]
- [x] 9.2 Change `crates/it/tests/db_roundtrip.rs` — `connect_back_udf_queries_and_emits`: (a) change DDL to `RUST SET SCRIPT connect_back_query() EMITS (result BIGINT)` with `%connection CB_SELF` and `%udf_object`; (b) change assertion query to `SELECT TO_CHAR(result) FROM (SELECT connect_back_query() EMITS (result BIGINT) FROM DUAL)`. Reorder: run `connect_back_dml_inserts_visible_via_exapump` BEFORE `connect_back_udf_queries_and_emits` so SET is proven first. [expert]
- [x] 9.3 Rebuild test UDFs: `cargo +1.91 build --release -p connect-back-query`; run `cargo fmt --check`; run `cargo +1.91 clippy --all-targets --all-features -- -D warnings`. [expert]
- [x] 9.4 Run integration suite `EXASOL_DB_SERIES=2026-1`; SIGABRT persists for connect-back scenarios (SCALAR→SET does not fix native-binary protocol crash). See verification-report.md §Root Cause Revision for full analysis. [expert]
- [x] 9.5 Update verification-report.md with findings; root cause revised (SCALAR→SET insufficient; fix requires WebSocket transport in exaudfclient). [expert]
