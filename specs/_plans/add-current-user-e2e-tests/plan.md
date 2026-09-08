# Plan: add-current-user-e2e-tests

## Summary

Verify the `current_user`, `scope_user`, and `current_schema` handshake fields against a live Exasol database through five new integration scenarios, and replace 20 hand-written `impl UdfContext` test doubles with one `TestContext` shipped behind a new non-default `test-support` feature of `exasol-udf-sdk`.

## Design

### Context

The host threads `current_user`, `scope_user`, and `current_schema` from `exascript_info` through `HandshakeMeta` to the `UdfContext` accessors. Unit tests cover the synthetic path only. No test observes what the database actually puts in those fields, so the project has no evidence for the values a UDF author will read.

Primary source research changes the shape of the work. Exasol documents `scope_user` as "getCurrentUser() or the owner of a view if the udf script is called within a view". That definition fixes one invariant: absent a view, `scope_user` equals `current_user`. The two fields therefore cannot diverge under plain `IMPERSONATE`. Issue #92 predicted a divergence, with `current_user` holding the login user and `scope_user` holding the impersonated user. The documentation refutes that split.

The documentation settles nothing about the other half. Exasol documents no mapping from `IMPERSONATE` to `CURRENT_USER`. `EXA_DBA_SESSIONS` instead carries `USER_NAME` for the logged-in user and `EFFECTIVE_USER` for the impersonated user, so either value is a plausible source for the field. Which user the field names stays unverified until the live run. The IMPERSONATE scenario therefore discovers the mapping, cross-checking the reported value against `SELECT CURRENT_USER` and both `EXA_DBA_SESSIONS` columns. The plan adds the view-owner case as the one database configuration that separates the two fields.

The second half of the issue is duplication. A survey of the repository found 22 `impl UdfContext` blocks in test code behind only 7 distinct behaviors. Adding a required trait method costs 22 edits today.

- **Goals**: prove the three identity fields end to end against a live database, give UDF authors and in-repo fixtures one `UdfContext` double, and document the database-side semantics in `protocol/handshake`.
- **Non-Goals**: no change to the `UdfContext` accessor signatures, to `HostContextBridge`, or to the ABI vtable. No change to the `current-user-meta` fixture's wire format. No new IT harness helper for user provisioning.

### Decision

#### Architecture

```
 spec: protocol/handshake            spec: sdk/udf-sdk
        (DB semantics)                (test-support)
             │                              │
             ▼                              ▼
 crates/it/tests/db_roundtrip.rs   exasol-udf-sdk/src/test_support.rs
   5 scenario fns, 2 connections     TestContext + DefaultsCtx
             │                              │
             ▼                              ▼
 test-udfs/current-user-meta        20 migrated call sites
   (cdylib, DB dlopens it)           (13 test-udfs, 2 SDK, 3 macros, 1 runtime)
```

Live-database session state is the scarce resource. `db_roundtrip_all_scenarios` is one `#[tokio::test]` that shares a single connection across every scenario. It registers the SLC with a session-scoped `ALTER SESSION SET SCRIPT_LANGUAGES` and holds `OPEN SCHEMA it_rust`. Four of the five new scenarios mutate that session state.

`IMPERSONATE` has no withdraw statement. A session reverts only by impersonating the original user. That revert needs `GRANT IMPERSONATION ON SYS` issued before the first `IMPERSONATE`.

The plan therefore splits the scenarios by whether they mutate session state. The baseline scenario runs on the shared connection like every existing scenario. The open-schema, absent-schema, view-owner, and `IMPERSONATE` scenarios run on one dedicated side connection that registers its own SLC. The file already sets this precedent: the `python3_connect_back` diagnostic opens a throwaway connection so a failure "cannot poison the shared `conn`".

The order on that side connection is load-bearing. The `IMPERSONATE` scenario runs last and closes the connection, so no later scenario reads or writes under a foreign identity. It still runs `IMPERSONATE SYS` before the close, which restores the login identity on the documented revert path. Ordering carries correctness. The revert is a second safeguard, not the mechanism the design depends on, because Exasol documents no guarantee that a reverted session equals a fresh `SYS` session.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Dedicated side connection for state-mutating scenarios, `IMPERSONATE` last on it | `crates/it/tests/db_roundtrip.rs` | A mid-scenario failure would leave a shared session in an unknown identity. `IMPERSONATE` reverts only via `IMPERSONATE SYS`, so the scenario restores the identity, then closes the connection |
| Two-phase read on one session | IMPERSONATE scenario | Reading before and after inside one session leaves the impersonation as the only changed variable |
| Deep double with policy enums, not per-mock knobs | `test_support::TestContext` | Two constructors plus three policy enums cover all 7 surveyed variants |
| Deliberately minimal second double | `test_support::DefaultsCtx` | A double that overrides a provided method shadows the default that test asserts |
| Fixture renders `<none>` for an absent `Option` | `test-udfs/current-user-meta` | Keeps an omitted database field distinguishable from an empty one over a text channel |

#### Design Diagnostic

`test_support` is the only new module and `TestContext`/`DefaultsCtx` the only new interfaces, so both answer every question in `/speq:design-philosophy`'s Quick Diagnostic.

| Question | Answer |
|----------|--------|
| Does a one-sentence summary capture what the module is responsible for? | Yes: it supplies `UdfContext` implementations for tests that have no host. |
| Is calling the module noticeably easier than reimplementing it would be? | Yes: `TestContext::scalar(row)` replaces a struct plus four method bodies, roughly 25 lines, at each of 21 sites. |
| Would changing how the module works internally force an edit outside it? | No: callers touch two constructors, three policy setters, and two accessors. The cursor, the emitted-row buffer, and the out-of-range error stay private. |
| Does a public doc comment explain the reasoning, not only restate the name? | Yes, task 1.5 requires the rustdoc to name the shadowing hazard that justifies two doubles rather than one. |
| Is there exactly one module that owns each significant design decision? | Yes: `test_support` owns "what a test double does", and the `<none>` render marker stays owned by the `current-user-meta` fixture alone. |
| Could a newcomer tell where the module ends without reading its internals? | Yes: it is gated behind one feature, exports two types, and depends on nothing but `UdfContext`, `Value`, and `UdfError`. |
| Does any tactical shortcut have a scheduled follow-up? | The two bespoke doubles in `crates/exasol-udf-sdk/tests/` are a stated permanent exception, not a shortcut. Task 2.8 records the reason in each file so no later reader treats it as an oversight. |
| Does business logic depend only inward? | Yes: `test_support` depends on the SDK's own traits and value types only. It links no host runtime, no `arrow`, and no test framework. |

The module is deep: seven surveyed behaviors collapse into two constructors plus three policy enums, so the interface a caller learns is far smaller than the code it removes.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Assert the documented invariant under `IMPERSONATE`, discover the `current_user` mapping, and add a view-owner scenario for the split | Assert issue #92's prediction that `IMPERSONATE` splits the two fields | The `getScopeUser()` definition refutes the split: absent a view the two fields cannot diverge. Exasol documents no mapping from `IMPERSONATE` to `CURRENT_USER`, so the mapping stays unverified until the live run and the scenario records it. |
| `current_schema` open-schema and cross-schema cases become one scenario, plus a new absent-schema scenario | Two separate scenarios as issue #92 lists them | Issue scenarios 3 and 4 assert the same inequality with the same setup. The absent case is the only end-to-end coverage of the `optional` field being absent. |
| Non-default `test-support` feature gated `#[cfg(any(test, feature = "test-support"))]` | Always-on module; `default = ["test-support"]`; `#[cfg(test)]` only | The interview chose a real non-default feature. The `any(test, ...)` arm lets the SDK's own unit tests use the double without a self dev-dependency. |
| `crates/exasol-udf-sdk/tests/connect_back.rs` and `tests/feature_gate.rs` keep their bespoke doubles | Migrate them too | An integration test in `tests/` sees only the feature set the resolver picked for the package, which differs between `cargo test` and `cargo test -p exasol-udf-sdk`. Both files also assert trait defaults. |
| One canonical out-of-range `get` error (`UdfError::Type`) with no caller knob | Expose the error kind and message as builder knobs | The surveyed mocks disagree on kind and message and no test asserts either. A knob here would leak a test detail into the interface. |
| Inline `CREATE USER`/`GRANT`/`DROP USER` in the scenario functions | A reusable `Harness` helper | The interview chose inline SQL, matching how other scenarios do their own setup. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| protocol/handshake | CHANGED | `specs/_plans/add-current-user-e2e-tests/protocol/handshake/spec.md` |
| examples/test-udfs | CHANGED | `specs/_plans/add-current-user-e2e-tests/examples/test-udfs/spec.md` |
| sdk/udf-sdk | CHANGED | `specs/_plans/add-current-user-e2e-tests/sdk/udf-sdk/spec.md` |

## Impact

`exasol-udf-sdk` gains one non-default feature, `test-support`, and two public types under it. No breaking change: the feature adds items only, enables no other feature, and adds no `UdfContext` method, so the `dyn UdfContext` vtable layout and the ABI fingerprint stay unchanged. UDF authors gain a documented way to unit-test a UDF without hand-writing a `UdfContext`. The integration suite gains one fixture and five scenarios, extending an IT run by four database round trips plus one extra session. Operators see no change.

## Requirements

| Requirement | Details |
|-------------|---------|
| `IMPERSONATE` privilege sequence | The `SYS` session needs `IMPERSONATION ON <user>` or the `IMPERSONATE ANY USER` system privilege. Exasol does not document either as implicit for the DBA role, so task 3.7 grants the object privilege explicitly rather than relying on `SYS`. |
| `IMPERSONATE` revert path | The scenario MUST run `GRANT IMPERSONATION ON SYS TO IT_IMPERSONATED` before the first `IMPERSONATE`, so the session returns with `IMPERSONATE SYS`. Exasol states: "There is no statement for withdrawing an impersonation. To revert to the user that initiated the session, the user must impersonate again as the original user." Source: https://docs.exasol.com/db/latest/sql/impersonate.htm |
| View owner privileges on the wrapped script | Exasol checks a view's underlying query against the view owner: "The owner of v must possess the privileges required to run the underlying query in v." So `IT_VIEW_OWNER` MUST hold `USAGE ON SCHEMA IT_RUST` and `EXECUTE ON SCRIPT IT_RUST.CURRENT_USER_META`. Source: https://docs.exasol.com/db/latest/database_concepts/privileges/details_rights_management.htm |
| No open write locks before `IMPERSONATE` | Exasol rejects `IMPERSONATE` while the transaction holds write locks. `exarrow-rs` autocommits each statement, which satisfies this, and the scenario MUST NOT batch DDL into an open transaction. |
| Side connection registers its own SLC | `ALTER SESSION SET SCRIPT_LANGUAGES` is session-scoped, so the side connection calls `it::register_slc` before it creates or selects any Rust script. |
| `test-support` enables nothing else | `test-support = []`. Any implied feature would change the featureless test configuration that `exa-udf-runtime` depends on for emit-arrow-off coverage. |
| Version bump and lock file | `[workspace.package].version` moves `0.23.0` to `0.24.0` (MINOR, new feature). The pinned `[workspace.dependencies] exasol-udf-sdk` version tracks it and `Cargo.lock` is regenerated in the same commit. |
| Test-only privilege scope | The three users and two schemas the scenarios create live only in the throwaway Exasol Docker container that `Harness::start` boots and discards per run. No scenario grants a privilege on a durable database, and every scenario drops what it created. |

## Dependencies

The `test-udfs/current-user-meta/` directory and the workspace `Cargo.toml` `members`/`default-members` entries exist as uncommitted local work. Task 3.1 and task 3.2 land them. Treat neither as done.

## Migration

| Current | New |
|---------|-----|
| 22 hand-written `impl UdfContext` blocks in test code | 2 documented bespoke blocks plus 21 uses of `TestContext`/`DefaultsCtx` |
| Fixture unit test declares its own `TestCtx`/`MetaCtx` struct | Fixture unit test constructs `TestContext` from the SDK `test-support` feature |

## Implementation Tasks

1. SDK test-support surface
    1. Add `test-support = []` to `[features]` in `crates/exasol-udf-sdk/Cargo.toml` and declare `#[cfg(any(test, feature = "test-support"))] pub mod test_support;` in `crates/exasol-udf-sdk/src/lib.rs`.
    2. Write failing unit tests in `crates/exasol-udf-sdk/src/test_support_tests.rs` for the scalar constructor, the set cursor, `emitted()`, `captured_return()` distinguishing not-called from `Some(None)`, the emit and next error policies, the metadata setters, and both out-of-range `get` paths.
    3. Implement `TestContext` in `crates/exasol-udf-sdk/src/test_support.rs` covering all 7 surveyed mock behaviors with two constructors and three policy enums, and no out-of-range-error knob. [expert]
    4. Implement `DefaultsCtx` in the same module plus a unit test asserting it overrides no provided method, checking `memory_limit`, `session_id`, `current_user`, `debug_level`, and `set_return`.
    5. Write rustdoc on `test_support`, `TestContext`, and `DefaultsCtx` that states why two doubles exist, naming the shadowing hazard `DefaultsCtx` avoids.
2. Migrate the existing test doubles
    1. Add `exasol-udf-sdk = { path = "...", features = ["test-support"] }` to `[dev-dependencies]` in `crates/exasol-udf-macros`, `crates/exa-udf-runtime`, and the 13 `test-udfs` crates that hold doubles.
    2. Replace the 3 migratable zero-column stubs with `DefaultsCtx`: `crates/exasol-udf-sdk/src/context_tests.rs` `DummyCtx`, `crates/exasol-udf-macros/tests/run_error.rs` `NoopCtx`, `crates/exasol-udf-macros/tests/vs_adapter.rs` `NoopCtx`.
    3. Replace the 2 fixed-debug-level doubles: `crates/exasol-udf-sdk/src/lib_tests.rs` `FixedLevelCtx`, `crates/exa-udf-runtime/tests/debug_level.rs` `FixedCtx`.
    4. Replace `crates/exasol-udf-macros/tests/output_shape.rs` `RecordingCtx`, keeping the `Option<Option<Value>>` comparison that separates not-called from called-with-`None`.
    5. Replace the 8 scalar flat-row doubles: `context_tests.rs` `TypedDummyCtx`, and the `TestCtx` in `emit-k`, `json-parse`, `resolv-udf`, `scalar-double`, `timestamp-add-second`, `timestamp-passthrough`, `scalar-next-illegal`.
    6. Replace the 4 SET cursor doubles: the `TestCtx` in `emit-bulk`, `numeric-temporal-ingest`, `set-filter`, `set-sum`.
    7. Replace `test-udfs/handshake-meta` `MetaCtx` and `test-udfs/returns-with-emit` `TestCtx`.
    8. Add a comment in `crates/exasol-udf-sdk/tests/connect_back.rs` and `tests/feature_gate.rs` stating why each keeps a bespoke double, then confirm `grep -rn "UdfContext for"` reports only `rowset.rs`, `test_support.rs`, and those two files.
3. Fixture, CI wiring, and live-database scenarios
    1. Commit `test-udfs/current-user-meta/` and add its `#[cfg(test)] #[path = "lib_tests.rs"] mod tests;` unit test built on `TestContext`, asserting the pipe join order and the `<none>` marker.
    2. Commit the workspace `Cargo.toml` `members` and `default-members` entries for `test-udfs/current-user-meta`, and add `-p current-user-meta` to the CI "Build UDF .so artifacts (release)" allowlist in `.github/workflows/ci.yml`.
    3. Add `const CURRENT_USER_META_LIB: &str = "libcurrent_user_meta.so";` and its `harness.upload_udf` call to the artifact-upload block of `db_roundtrip_all_scenarios`.
    4. Add `current_user_meta_reports_session_user_and_open_schema`, following the structure of `handshake_metadata_udf_emits_session_and_node`, with a per-field comment stating why the assertion distinguishes a live value from the accessor default.
    5. Add `current_user_meta_current_schema_tracks_open_schema` and `current_user_meta_absent_current_schema` on a dedicated side connection that calls `it::register_slc` first, creating `IT_RUST_OTHER`, selecting the script by qualified name, and restoring `OPEN SCHEMA IT_RUST` after each. Leave the connection open for tasks 3.6 and 3.7.
    6. Add `current_user_meta_scope_user_is_view_owner` on the same side connection, in this statement order: `CREATE USER IT_VIEW_OWNER`, `GRANT USAGE ON SCHEMA IT_RUST TO IT_VIEW_OWNER`, `GRANT EXECUTE ON SCRIPT IT_RUST.CURRENT_USER_META TO IT_VIEW_OWNER`, `CREATE SCHEMA IT_VIEW_SCOPE`, create the wrapping view as `SYS`, `ALTER SCHEMA IT_VIEW_SCOPE CHANGE OWNER IT_VIEW_OWNER`, select from the view as `SYS`, then drop the view, the schema, and the user. Create the view before the owner change, because `ALTER SCHEMA ... CHANGE OWNER` is the documented path that re-owns "the schema and all its objects", while no page documents the owner of a view `SYS` creates in a foreign-owned schema. Grant the view owner both privileges, because Exasol checks the view's underlying query against its owner. `functions/alphabeticallistfunctions/scope_user.htm` ships `CREATE VIEW scope_view AS SELECT SCOPE_USER;` as an official example, so a `FROM`-less view body is supported. Wrap the call over a one-row source only as a fallback. Leave the connection open for task 3.7. [expert]
    7. Add `current_user_meta_follows_impersonate` as the last scenario on the side connection: read the metadata and `SELECT CURRENT_SESSION` as `SYS`, then `CREATE USER IT_IMPERSONATED`, `GRANT EXECUTE ON SCRIPT IT_RUST.CURRENT_USER_META TO IT_IMPERSONATED`, `GRANT IMPERSONATION ON IT_IMPERSONATED TO SYS`, `GRANT IMPERSONATION ON SYS TO IT_IMPERSONATED`, `IMPERSONATE IT_IMPERSONATED`, and re-read. Cross-check the reported `current_user` two ways: it MUST equal `SELECT CURRENT_USER` run in the same impersonated session, and it MUST equal exactly one of `USER_NAME` or `EFFECTIVE_USER` from `EXA_DBA_SESSIONS` for the recorded session id. Read `EXA_DBA_SESSIONS` on the shared `SYS` connection, because the impersonated user "loses all their current privileges" and may not reach that system table. Record in a code comment which column matched, and assert nothing about the direction beyond those two equalities. Then run `IMPERSONATE SYS`, assert `current_user` reports `SYS` again, close the side connection, and `DROP USER IT_IMPERSONATED CASCADE` on the shared connection. Start from that least-privilege grant set. Widen to `GRANT DBA` only if the impersonated user cannot reach the SLC in BucketFS, and record which grant set the live database required in a code comment. [expert]
    8. Wire the five scenario calls and their `eprintln!("[it] scenario ... ok")` lines into `db_roundtrip_all_scenarios`. Call the baseline scenario on the shared connection first, then open the side connection after the artifact uploads and call, in this fixed order, `current_user_meta_current_schema_tracks_open_schema`, `current_user_meta_absent_current_schema`, `current_user_meta_scope_user_is_view_owner`, and last `current_user_meta_follows_impersonate`, which closes the side connection. Place the whole block before the existing connect-back scenarios. The order is load-bearing: only the last scenario impersonates, so no other scenario runs under a foreign identity or on a closed connection. Wrap each side-connection scenario in the `harness.dump_udf_logs()` failure branch the connect-back scenarios already use, because a UDF crash inside an impersonated session leaves no other diagnostic.
4. Release hygiene
    1. Bump `[workspace.package].version` to `0.24.0`, update the pinned `[workspace.dependencies] exasol-udf-sdk` version to match, and commit the regenerated `Cargo.lock`.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: SDK test-support surface | 1.1-1.5 | — | spec delta `sdk/udf-sdk`; `crates/exasol-udf-sdk/Cargo.toml`, `crates/exasol-udf-sdk/src/lib.rs`, `crates/exasol-udf-sdk/src/test_support.rs`, `crates/exasol-udf-sdk/src/test_support_tests.rs` |
| B: Test-double migration | 2.1-2.8 | A (consumes `test_support`) | no spec delta (test mechanics, see decision 5); `crates/exasol-udf-macros/tests/{output_shape,run_error,vs_adapter}.rs`, `crates/exasol-udf-sdk/src/{context_tests,lib_tests}.rs`, `crates/exasol-udf-sdk/tests/{connect_back,feature_gate}.rs`, `crates/exa-udf-runtime/tests/debug_level.rs`, the 13 `test-udfs/*/src/lib.rs` doubles, and those 15 crates' `Cargo.toml` files |
| C: Identity fixture and live-DB scenarios | 3.1-3.8 | A (fixture unit test uses `TestContext`) | spec deltas `protocol/handshake`, `examples/test-udfs`; `test-udfs/current-user-meta/`, workspace `Cargo.toml`, `.github/workflows/ci.yml`, `crates/it/tests/db_roundtrip.rs` |
| D: Release hygiene | 4.1 | A, B, C (shares workspace `Cargo.toml` with C, so runs strictly after it) | workspace `Cargo.toml`, `Cargo.lock` |

B and C run in parallel after A. They share no file. B edits per-crate `Cargo.toml` files of the 15 crates holding doubles. C edits the workspace root `Cargo.toml` and the new fixture only. Inside `crates/exasol-udf-sdk/src/`, A owns `lib.rs`, `test_support.rs`, and `test_support_tests.rs`, while B owns `context_tests.rs` and `lib_tests.rs` and never edits `test_support.rs`.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Struct + impl | `crates/exasol-udf-sdk/src/context_tests.rs` `DummyCtx`, `TypedDummyCtx` | Replaced by `DefaultsCtx` and `TestContext` |
| Struct + impl | `crates/exasol-udf-sdk/src/lib_tests.rs` `FixedLevelCtx` | Replaced by `TestContext` with a set debug level |
| Struct + impl | `crates/exasol-udf-macros/tests/output_shape.rs` `RecordingCtx` | Replaced by `TestContext` return recording |
| Struct + impl | `crates/exasol-udf-macros/tests/run_error.rs` `NoopCtx`, `tests/vs_adapter.rs` `NoopCtx` | Replaced by `DefaultsCtx` |
| Struct + impl | `crates/exa-udf-runtime/tests/debug_level.rs` `FixedCtx` | Replaced by `TestContext` with a set debug level |
| Struct + impl | `TestCtx` in `test-udfs/{emit-bulk,emit-k,json-parse,numeric-temporal-ingest,resolv-udf,returns-with-emit,scalar-double,scalar-next-illegal,set-filter,set-sum,timestamp-add-second,timestamp-passthrough}/src/lib.rs` | Replaced by `TestContext` |
| Struct + impl | `test-udfs/handshake-meta/src/lib.rs` `MetaCtx` | Replaced by `TestContext` metadata setters |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Identity metadata reports the executing user and the open schema | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `current_user_meta_reports_session_user_and_open_schema` |
| current_schema tracks the open schema independently of script_schema | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `current_user_meta_current_schema_tracks_open_schema` |
| A session with no open schema reports no current schema | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `current_user_meta_absent_current_schema` |
| scope_user reports the view owner when the script runs inside a view | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `current_user_meta_scope_user_is_view_owner` |
| IMPERSONATE establishes which user the current_user field reports | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `current_user_meta_follows_impersonate` |
| current-user-meta reports the session identity fields as one string | Unit | `test-udfs/current-user-meta/src/lib_tests.rs` | `current_user_meta_joins_five_fields_and_marks_absent_optionals` |
| The test-support feature ships a reusable UdfContext test double | Unit | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `test_context_covers_scalar_set_emit_and_return_paths` |
| The test-support feature ships a defaults-preserving UdfContext double | Unit | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `defaults_ctx_overrides_no_provided_method` |

Both SDK scenarios are pure computation with no I/O, so they take unit tests. The fixture-format scenario is pure string formatting over a `TestContext`, so it takes a unit test. The baseline integration scenario additionally observes that fixture's live wire format.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| examples/test-udfs | `cargo build --release -p current-user-meta` | Exit 0, `target/release/libcurrent_user_meta.so` present |
| sdk/udf-sdk | `cargo test -p exasol-udf-sdk --features test-support` | 0 failures, the `test_support_tests` cases run |
| sdk/udf-sdk | `cargo build -p exasol-udf-sdk` | Exit 0 with no `test_support` symbols, proving the feature gate holds |
| sdk/udf-sdk | `cargo test -p exasol-udf-sdk` | 0 failures, proving `tests/feature_gate.rs` still compiles without the feature |
| protocol/handshake | `cargo test -p it --features integration -- --nocapture` | `[it] scenario current_user_meta_* ok` printed five times |
| protocol/handshake | `cargo test -p it --features integration` after removing `-p current-user-meta` from CI | Fails with `reading UDF artifact .../libcurrent_user_meta.so`, proving the allowlist entry is load-bearing |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Test (all features) | `cargo test --all-features` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
