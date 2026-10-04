# Decisions: add-current-user-e2e-tests

## ADR: Two doubles, not one: DefaultsCtx exists to avoid shadowing the trait defaults

**ID:** two-udfcontext-test-doubles-avoid-shadowing
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

Tests assert the default bodies of provided `UdfContext` methods. A full-featured double overrides those methods and shadows the defaults, and Rust cannot conditionally skip an override.

### Decision

`test_support` exports `TestContext`, the full double, and `DefaultsCtx`, which implements only the four required methods and overrides no provided method.

### Options Considered

| Option | Verdict |
|--------|---------|
| `TestContext` and `DefaultsCtx` | ✓ Chosen |
| One `TestContext` for every site | ✗ Default-asserting tests would verify the double's own re-implementation |

### Consequences

Callers pick the double that matches the assertion.

## ADR: The test-support feature enables no other feature and is gated any(test, feature)

**ID:** test-support-feature-no-implied-features
**Plan:** add-current-user-e2e-tests
**Status:** Accepted

### Context

Dev-dependency features unify into the crate under test, as `crates/exa-udf-runtime/Cargo.toml` documents. A `test-support` feature implying another feature would break that crate's featureless emit-arrow-off coverage.

### Decision

`test-support = []`, declared with `#[cfg(any(test, feature = "test-support"))] pub mod test_support;`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Empty feature, gated `any(test, feature)` | ✓ Chosen |
| Always-on module, or a default feature | ✗ A non-default feature is required |
| Self dev-dependency on `exasol-udf-sdk` | ✗ Links two incompatible copies of every type |

### Consequences

`test-support` adds items only and changes no other feature's behavior.
