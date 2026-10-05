# Decisions: add-current-user-e2e-tests

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
