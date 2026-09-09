# Decisions: add-current-user-e2e-tests

## ADR: Assert the documented invariant under IMPERSONATE and discover the current_user mapping

**ID:** assert-impersonate-current-user-mapping-invariant
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

Issue #92 predicted that `IMPERSONATE` splits `current_user` (login user) from `scope_user` (impersonated user). Exasol documents the UDF metadata field as `getScopeUser()`: "Scope user (getCurrentUser() or the owner of a view if the udf script is called within a view)", which refutes the split absent a view. Exasol documents no mapping from `IMPERSONATE` to `CURRENT_USER` or to the UDF `current_user` field. `EXA_DBA_SESSIONS` documents a two-user model, `USER_NAME` for the logged-in user and `EFFECTIVE_USER` for the impersonated user, so either column is a plausible source for the field.

### Decision

The IMPERSONATE scenario asserts one documented invariant: `scope_user` equals `current_user` absent a view, and `script_schema` stays unchanged. It discovers which user `current_user` names by cross-checking the reported value against `SELECT CURRENT_USER` in the same impersonated session, and against `EXA_DBA_SESSIONS.USER_NAME` and `EXA_DBA_SESSIONS.EFFECTIVE_USER` read for that session. The scenario records the observed mapping instead of presupposing it.

### Options Considered

| Option | Verdict |
|--------|---------|
| Assert the documented invariant and discover the current_user mapping via cross-check | ✓ Chosen — matches only the evidence Exasol documents; asserts no unverified direction |
| Encode issue #92's prediction as written (current_user stays login user, scope_user becomes impersonated user) | ✗ Rejected — the `getScopeUser()` definition refutes the field split; the other half is neither confirmed nor refuted by any Exasol page |

### Consequences

The spec records only what the documentation and the live cross-check support, avoiding an unevidenced database fact in the permanent library. The scenario carries extra verification steps (two cross-checks) instead of one direct assertion.

## ADR: Add a view-owner scenario as the only case that separates scope_user from current_user

**ID:** add-view-owner-scope-user-scenario
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

After the IMPERSONATE-mapping decision, none of issue #92's four listed scenarios ever separates `scope_user` from `current_user`, leaving that field with no differential end-to-end coverage, which was the issue's stated goal.

### Decision

Add a fifth scenario in which a view owned by a second user wraps the UDF call, so `current_user` reports the executing user and `scope_user` reports the view owner.

### Options Considered

| Option | Verdict |
|--------|---------|
| Add a view-owner scenario | ✓ Chosen — the view wrapper is the single documented configuration where the two fields differ |
| Ship only the four scenarios issue #92 lists | ✗ Rejected — proves only that the two fields are equal, indistinguishable from the host copying one field into both |

### Consequences

The suite gains real differential coverage of `scope_user`, at the cost of one more scenario with view and cross-schema-ownership setup.

## ADR: Run state-mutating scenarios on a dedicated side connection

**ID:** run-state-mutating-scenarios-side-connection
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

`db_roundtrip_all_scenarios` is one `#[tokio::test]` sharing a session-scoped `ALTER SESSION SET SCRIPT_LANGUAGES` and `OPEN SCHEMA it_rust` across roughly 30 scenarios. A mid-scenario failure in a scenario that changes session identity or schema would leave the shared session in an unknown state, cascading failures across unrelated later scenarios. The file already isolates the `python3_connect_back` diagnostic on its own throwaway connection for the same reason.

### Decision

The baseline scenario runs on the shared `conn`. The open-schema, absent-schema, view-owner, and IMPERSONATE scenarios run on one dedicated connection that registers its own SLC. The IMPERSONATE scenario runs last on that connection, restores the identity with `IMPERSONATE SYS`, and closes it.

### Options Considered

| Option | Verdict |
|--------|---------|
| Dedicated side connection, IMPERSONATE last, closed after | ✓ Chosen — isolates a mid-scenario failure from the roughly 30 other scenarios on the shared connection |
| Run everything on the shared connection with explicit restore steps | ✗ Rejected — a failure mid-scenario still leaves the shared session in an unknown identity |

### Consequences

Ordering carries the correctness of the isolation; the `IMPERSONATE SYS` restore is a second safeguard only, because Exasol documents no guarantee that a reverted session equals a fresh `SYS` session.

## ADR: The published test-support surface is specified, the in-repo mock migration is not

**ID:** spec-test-support-surface-not-mock-migration
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

`exasol-udf-sdk` is published to crates.io. `test-support` is author-facing API: a UDF author enables it as a dev-dependency feature to unit-test their own UDF. Migrating the 20 in-repo hand-written `UdfContext` doubles to use it is an internal mechanics concern, not a published capability.

### Decision

Add two scenarios to `sdk/udf-sdk` specifying `TestContext` and `DefaultsCtx`. Add no spec scenario for migrating the 20 in-repo doubles; that migration is recorded in CLAUDE.md/plan mechanics instead.

### Options Considered

| Option | Verdict |
|--------|---------|
| Spec the published test-support capability only | ✓ Chosen — matches the project rule that specs hold business requirements, not build/test mechanics |
| Spec neither, treating the whole item as test mechanics | ✗ Rejected — `test-support` is author-facing crates.io API, not harness plumbing |

### Consequences

The spec library states the author-facing guarantee precisely, while which in-repo file holds which double stays out of the permanent spec library, matching CLAUDE.md's scope rule.

## ADR: Two doubles, not one: DefaultsCtx exists to avoid shadowing the trait defaults

**ID:** two-udfcontext-test-doubles-avoid-shadowing
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

Six existing tests assert the default bodies of provided `UdfContext` methods, including `default_memory_limit_is_zero`, `default_set_return_unimplemented`, `default_handshake_metadata_is_neutral`, and `default_debug_level_is_info`. A single full-featured double that overrides those methods would shadow the trait defaults those tests exist to verify, and Rust offers no way to conditionally not override a method.

### Decision

`test_support` exports `TestContext` (the full double, covering all seven surveyed mock behaviors) and `DefaultsCtx` (only the four required methods, overriding no provided method).

### Options Considered

| Option | Verdict |
|--------|---------|
| Two narrow doubles: TestContext and DefaultsCtx | ✓ Chosen — keeps default-asserting tests honest while collapsing 5 near-identical stubs into DefaultsCtx |
| One TestContext for every site | ✗ Rejected — would silently verify the double's re-implementation instead of the trait's own defaults |

### Consequences

Callers must pick the right double for the assertion they are making, but no test can be fooled by a double's re-implementation of a default.

## ADR: The test-support feature enables no other feature and is gated any(test, feature)

**ID:** test-support-feature-no-implied-features
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

`crates/exa-udf-runtime/Cargo.toml` documents that dev-dependency features unify into the crate under test, which is why its emit-arrow fixture is an optional normal dependency rather than a feature-gated one. A `test-support` feature that implied any other feature would break that crate's featureless emit-arrow-off coverage.

### Decision

`test-support = []`, declared with `#[cfg(any(test, feature = "test-support"))] pub mod test_support;`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Empty feature list, gated any(test, feature) | ✓ Chosen — lets in-crate unit tests use the doubles with no dev-dependency, and implies nothing that could break featureless coverage elsewhere |
| Always-on module, or a default feature | ✗ Rejected — the interview chose a real non-default feature |
| Self dev-dependency on exasol-udf-sdk to enable the feature for its own tests | ✗ Rejected — links a second copy of the crate, yielding two incompatible copies of every type |

### Consequences

Any dependent crate's featureless test configuration keeps its meaning; `test-support` adds items only and changes no other feature's behavior.
