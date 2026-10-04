# Decisions: add-current-user-e2e-tests

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
