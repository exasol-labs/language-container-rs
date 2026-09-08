# Decision Log: add-current-user-e2e-tests

## Interview

**Q:** Issue #92 bundles a Primary piece (E2E IMPERSONATE tests) and a Secondary piece (a `TestContext` builder that de-duplicates 15+ mock `UdfContext` impls). Should this plan cover both, or just the primary?
**A:** Both in one plan.

**Q:** Since test-udfs and macro tests live in separate crates, not just in `#[cfg(test)]` inside `exasol-udf-sdk`, how should the `TestContext` builder be exposed?
**A:** A cargo feature `test-support`: a real, non-default feature flag that other crates enable as a dev-dependency feature, working across crate boundaries.

**Q:** IMPERSONATE needs a second DB user in the Exasol container. Should creating and cleaning up the impersonation-target user be a reusable Harness helper, or inline SQL local to the new scenario?
**A:** Inline SQL in the scenario. `CREATE USER` / `GRANT` / `DROP USER` written directly in the new IMPERSONATE test function only, matching how one-off SQL setup is done today, for example `handshake_metadata_udf_emits_session_and_node` in `crates/it/tests/db_roundtrip.rs`.

**Q:** Where should the DB-side semantics documentation live?
**A:** `specs/protocol/handshake`, grouped with the wire-level `HandshakeMeta` scenarios, as a protocol-level fact about what `exascript_info` carries rather than an SDK-accessor fact. The accessor signatures are already specified in `specs/sdk/udf-sdk` under "UdfContext exposes handshake identity and origin metadata".

**Q:** How far should the TestContext migration go: all 15+ existing call sites, or introduce-only?
**A:** Migrate all existing call sites. Full de-duplication now, one coherent PR, no leftover copy-pasted mocks.

## Design Decisions

### [1] Assert the documented invariant under IMPERSONATE and discover the current_user mapping

- **Decision:** The IMPERSONATE scenario asserts one documented invariant, that `scope_user` equals `current_user` absent a view, and that `script_schema` stays unchanged. It discovers which user `current_user` names by cross-checking the reported value against `SELECT CURRENT_USER` in the same impersonated session, and against `EXA_DBA_SESSIONS.USER_NAME` and `EXA_DBA_SESSIONS.EFFECTIVE_USER` read for that session. The scenario records the observed mapping instead of presupposing it.
- **Alternatives:** Encode issue #92's prediction as written, that `current_user` stays the login user while `scope_user` becomes the impersonated user. Rejected in one half only: the `getScopeUser()` definition refutes the field *split*, because absent a view the two fields cannot diverge. The other half, which user `current_user` names, is neither confirmed nor refuted by any Exasol page, so the plan asserts no direction there.
- **Rationale:** Exasol documents the UDF metadata field as `getScopeUser()`: "Scope user (getCurrentUser() or the owner of a view if the udf script is called within a view)". Exasol documents no mapping from `IMPERSONATE` to `CURRENT_USER` or to the UDF `current_user` field. `EXA_DBA_SESSIONS` instead documents a two-user model: `USER_NAME` is "Name of the logged-in user" and `EFFECTIVE_USER` is "Effective user of the session (changeable via IMPERSONATE)". Either column is a plausible source for the field, so a normative `MUST` on either would record an unevidenced database fact. Sources: https://docs.exasol.com/db/latest/database_concepts/udf_scripts/java.htm, https://docs.exasol.com/db/latest/sql/impersonate.htm, and https://docs.exasol.com/db/latest/sql_references/system_tables/metadata/exa_dba_sessions.htm.
- **Promotes to ADR:** yes

### [2] Add a view-owner scenario as the only case that separates scope_user from current_user

- **Decision:** Add a fifth scenario in which a view owned by a second user wraps the UDF call, so `current_user` reports the executing user and `scope_user` reports the view owner.
- **Alternatives:** Ship only the four scenarios issue #92 lists. Rejected because, after decision 1, none of those four ever separates `scope_user` from `current_user`, leaving the field with no differential end-to-end coverage. That coverage was the issue's stated goal.
- **Rationale:** The view wrapper is the single documented configuration where the two fields differ. Without it the plan would prove only that the two fields are equal, which no reader could distinguish from the host copying one field into both.
- **Promotes to ADR:** yes

### [3] Run state-mutating scenarios on a dedicated side connection

- **Decision:** The baseline scenario runs on the shared `conn`. The open-schema, absent-schema, view-owner, and IMPERSONATE scenarios run on one dedicated connection that registers its own SLC. The IMPERSONATE scenario runs last on that connection, restores the identity with `IMPERSONATE SYS`, and closes it.
- **Alternatives:** Run everything on the shared connection with explicit restore steps. Rejected because a failure mid-scenario leaves the shared session in an unknown user identity, and that session carries roughly 30 later scenarios that would then fail for an unrelated reason.
- **Rationale:** `db_roundtrip_all_scenarios` is one `#[tokio::test]` sharing a session-scoped `ALTER SESSION SET SCRIPT_LANGUAGES` and `OPEN SCHEMA it_rust` across roughly 30 scenarios. The file already applies this pattern to the `python3_connect_back` diagnostic so a failure "cannot poison the shared `conn`". A documented revert exists: Exasol states the user "must impersonate again as the original user", so a side connection is not the only way to undo an impersonation. It is the way that stops one failure from cascading. Ordering carries the correctness of the side connection, and the `IMPERSONATE SYS` restore is a second safeguard, because Exasol documents no guarantee that a reverted session equals a fresh `SYS` session.
- **Promotes to ADR:** yes

### [4] Grant the IMPERSONATION object privilege explicitly rather than assume the DBA role carries it

- **Decision:** The scenario runs `GRANT IMPERSONATION ON IT_IMPERSONATED TO SYS` and `GRANT IMPERSONATION ON SYS TO IT_IMPERSONATED` before `IMPERSONATE`, the second one so the session can revert with `IMPERSONATE SYS`. Task 3.7 verifies the sequence against the live database and records the working form in a code comment.
- **Alternatives:** Rely on `SYS` holding `IMPERSONATE ANY USER` implicitly. Rejected because Exasol's reference names `IMPERSONATE ANY USER` or `IMPERSONATION ON <user/role>` as the requirement and states nothing about the DBA role holding either implicitly.
- **Rationale:** An explicit object grant is deterministic and self-documenting. The plan also records the transaction precondition: Exasol rejects `IMPERSONATE` while the transaction holds write locks, and `exarrow-rs` autocommits each statement, which satisfies it.
- **Promotes to ADR:** no

### [5] The published test-support surface is specified, the in-repo mock migration is not

- **Decision:** Add two scenarios to `sdk/udf-sdk` for `TestContext` and `DefaultsCtx`. Add no spec scenario for migrating the 20 in-repo doubles.
- **Alternatives:** Spec neither, treating the whole item as test mechanics. Rejected because `exasol-udf-sdk` is published to crates.io, so `test-support` is author-facing API, not harness plumbing. A UDF author enables it as a dev-dependency feature to unit-test their own UDF.
- **Rationale:** CLAUDE.md scopes specs to business requirements and puts test-harness mechanics in CLAUDE.md instead. The published capability "a UDF author can unit-test a UDF without hand-writing a `UdfContext`" is a requirement. Which in-repo file holds which double is mechanics.
- **Promotes to ADR:** yes

### [6] Two doubles, not one: DefaultsCtx exists to avoid shadowing the trait defaults

- **Decision:** `test_support` exports `TestContext` (the full double) and `DefaultsCtx` (only the four required methods, overriding no provided method).
- **Alternatives:** One `TestContext` for every site. Rejected because six existing tests assert the default bodies of provided `UdfContext` methods, including `default_memory_limit_is_zero`, `default_set_return_unimplemented`, `default_handshake_metadata_is_neutral`, and `default_debug_level_is_info`. `TestContext` overrides those methods, so those tests would silently verify the double's re-implementation instead of the trait. Rust offers no way to conditionally not override a method.
- **Rationale:** Two narrow doubles keep each assertion honest. `DefaultsCtx` alone collapses 5 near-identical stubs into one type.
- **Promotes to ADR:** yes

### [7] Two SDK integration-test doubles stay bespoke

- **Decision:** `crates/exasol-udf-sdk/tests/connect_back.rs` `MockCtx` and `tests/feature_gate.rs` `Ctx` keep their hand-written impls, with a comment stating why. The other 20 doubles migrate.
- **Alternatives:** Migrate all 22. Rejected on two grounds. First, a test in the crate's own `tests/` directory sees only the feature set the resolver picked for that package, and `test-support` is on under a workspace `cargo test` but off under `cargo test -p exasol-udf-sdk`, so referencing the module there compiles in one invocation and fails in the other. Second, both files exist to assert the unfeatured public surface and the trait's own defaults, which decision 6 already excludes from `TestContext`.
- **Rationale:** This is a principled exception with a stated cause, not a dropped ask. Each remaining double is a unit struct with four short method bodies.
- **Promotes to ADR:** no

### [8] The test-support feature enables no other feature and is gated `any(test, feature)`

- **Decision:** `test-support = []` with `#[cfg(any(test, feature = "test-support"))] pub mod test_support;`.
- **Alternatives:** An always-on module, rejected because the interview chose a real non-default feature. A default feature, rejected for the same reason. A self dev-dependency on `exasol-udf-sdk` to enable the feature for its own tests, rejected because it links a second copy of the crate and yields two incompatible copies of every type.
- **Rationale:** The `any(test, ...)` arm lets `src/context_tests.rs` and `src/lib_tests.rs` use the doubles with no dev-dependency. The empty feature list is load-bearing: `crates/exa-udf-runtime/Cargo.toml` documents that dev-dependency features unify into the crate under test, which is why its emit-arrow fixture is an optional normal dependency. A `test-support` feature that implied anything would break that crate's featureless emit-arrow-off coverage.
- **Promotes to ADR:** yes

### [9] TestContext exposes policy enums, not a knob per surveyed mock

- **Decision:** Two constructors (`scalar`, `set`) plus three policies (emit, next, return recording) plus metadata setters. No knob for the out-of-range `get` error kind or message.
- **Alternatives:** Expose every axis the survey found, including the out-of-range error kind and message template. Rejected as information leakage: the surveyed mocks disagree on `UdfError::Type` versus `UdfError::User` and on the message text, and no test asserts either, so the disagreement is accidental. `TestContext` picks `UdfError::Type` and migrated tests adopt it.
- **Rationale:** A configuration parameter is a decision the module declined to make. Fixing the out-of-range error keeps the interface cheaper to learn than the code it replaces. The set-mode `get` before the first `next` also becomes `Err` instead of the arithmetic-underflow panic all four current cursor mocks exhibit.
- **Promotes to ADR:** no

### [10] Issue scenarios 3 and 4 merge, and an absent-schema scenario replaces the freed slot

- **Decision:** One scenario covers both the open-schema and cross-schema cases. A new scenario covers a session with no open schema.
- **Alternatives:** Keep four `current_schema` scenarios as issue #92 lists them. Rejected because its scenarios 3 and 4 share the same setup and assert the same inequality between `current_schema` and `script_schema`.
- **Rationale:** The skill permits combining scenarios that share setup and assertions. The absent-schema case is the only end-to-end coverage of the `optional` proto field being absent, which no existing test reaches. Its assertion accepts either the `<none>` marker or an empty string, because the choice between omitting the field and sending it empty belongs to the database.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] The IMPERSONATE-to-current_user mapping was asserted without evidence

- **Finding:** `plan-reviewer` round 1 flagged `[INTENT_DRIFT]`. The plan overrode issue #92 on a citation that supports only half the override. The `getScopeUser()` definition refutes the field *split*, but no Exasol page states which user `CURRENT_USER` or the UDF `current_user` field reports after `IMPERSONATE`. `EXA_DBA_SESSIONS` documents a two-user model that makes the issue's reading plausible. Asserting `current_user MUST report IT_IMPERSONATED` risked recording a false database fact into the permanent spec library.
- **Direction change:** The claim now splits by evidence. `protocol/handshake/spec.md` keeps the documented invariant as the `MUST`, that `scope_user` equals `current_user` absent a view and `script_schema` stays unchanged. The scenario, renamed "IMPERSONATE establishes which user the current_user field reports", now requires the reported value to equal `SELECT CURRENT_USER` in the same impersonated session, and to equal exactly one of `EXA_DBA_SESSIONS.USER_NAME` or `EXA_DBA_SESSIONS.EFFECTIVE_USER` for that session. Task 3.7 adds those cross-check reads. Decision [1] drops "would fail against every Exasol version". `plan.md` § Design/Context and § Consequences state the mapping as unverified until the live run.
- **Promotes to ADR:** yes

### [plan-review] IMPERSONATE has a documented revert path

- **Finding:** `plan-reviewer` round 1 flagged `[UNSTATED_ASSUMPTION]`. Decision [3] rested on "Closing a connection is the only reliable way to undo an impersonation". The page the plan cites refutes it: "To revert to the user that initiated the session, the user must impersonate again as the original user."
- **Direction change:** `plan.md` § Requirements gains a row requiring `GRANT IMPERSONATION ON SYS TO IT_IMPERSONATED` before the first `IMPERSONATE`, so the session returns with `IMPERSONATE SYS`. Task 3.7 adds the restore, which stays a plan requirement rather than a spec step, because it protects the test session rather than describing database behavior. Decision [3] now rests on the surviving ground alone: a mid-scenario failure leaves the shared session in an unknown identity, and that session carries roughly 30 later scenarios. `plan.md` § Design/Architecture and § Patterns row 1 state the revert as a second safeguard, not the mechanism the design depends on.
- **Promotes to ADR:** no

### [plan-review] The view owner held no privilege on the wrapped script

- **Finding:** `plan-reviewer` round 1 flagged `[HIDDEN_DEPENDENCY]`. Exasol checks a view's underlying query against the view owner, and the old task 3.7 granted `IT_VIEW_OWNER` nothing, so the only differential `scope_user` scenario could not pass. The old statement order also changed the schema owner before creating the view, which no Exasol page documents as setting the view's owner.
- **Direction change:** Task 3.6 now grants `USAGE ON SCHEMA IT_RUST` and `EXECUTE ON SCRIPT IT_RUST.CURRENT_USER_META` to `IT_VIEW_OWNER`, and creates the view before `ALTER SCHEMA IT_VIEW_SCOPE CHANGE OWNER IT_VIEW_OWNER`. The scenario `GIVEN` carries both grants and the ordering. `plan.md` § Requirements gains a row citing `details_rights_management.htm`. The `FROM`-less hedge becomes a stated fallback, because `scope_user.htm` ships `CREATE VIEW scope_view AS SELECT SCOPE_USER;` as an official example.
- **Promotes to ADR:** no

### [plan-review] The side-connection scenario order contradicted itself

- **Finding:** `plan-reviewer` round 1 flagged `[REQUIREMENT_CONFLICT]`. The old task 3.5 closed the side connection, and the old tasks 3.6 and 3.7 then ran on it. Leaving it open instead ran `CREATE SCHEMA`, `CREATE USER`, and `ALTER SCHEMA` as the impersonated user, who "loses all their current privileges", and broke the view-owner scenario's `GIVEN a live session authenticated as SYS`.
- **Direction change:** The side-connection scenarios are renumbered. Task 3.5 covers open-schema and absent-schema, task 3.6 covers view-owner, and task 3.7 runs IMPERSONATE last and closes the connection. Task 3.8 states the order explicitly, and the `DELTA:NEW` scenario order in `protocol/handshake/spec.md` matches. Both review options are applied: the ordering makes correctness independent of the revert, and the `IMPERSONATE SYS` restore stays as a second safeguard, because Exasol documents no guarantee that a reverted session equals a fresh `SYS` session.
- **Promotes to ADR:** no
