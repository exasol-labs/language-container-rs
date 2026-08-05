# Decisions: refactor-rowset-dispatch-complexity

## ADR: CI coverage job hardened with a complete fixture list and an explicit feature set

**ID:** ci-coverage-fixture-list-explicit-feature-set
**Plan:** refactor-rowset-dispatch-complexity
**Status:** Accepted

### Context

The coverage job's "Build test fixture cdylibs" step built only 3 of the 8 fixtures the runtime test suite `dlopen`s. The job passed only because a warm cache carried the `build` job's `target/` output; on a cold cache it fails with a missing `.so`. The `cargo llvm-cov` invocation also measured whatever feature set the crate graph happened to unify onto the test build, inherited implicitly from `exaudfclient`'s `features = ["connect-back"]` declaration rather than declared by the coverage job itself.

### Decision

Extend the coverage job's fixture-build step to all 8 fixtures the runtime tests `dlopen` (`set-sum`, `emit-k`, `scalar-next-illegal`, `returns-with-emit`, `emit-arrow-batch`, plus the existing 3), and add `--all-features` to the `cargo llvm-cov` invocation so the measured feature set is explicit rather than accidental.

### Options Considered

| Option | Verdict |
|--------|---------|
| Complete fixture list + explicit `--all-features` | ✓ Chosen — makes the job self-sufficient on a cold cache and pins the measured feature set |
| Leave the 3-fixture list | ✗ Rejected — works only while the cache happens to carry `build`'s `target/`; fails cold with a missing `.so` |
| Omit `--all-features` | ✗ Rejected — leaves the measured feature set implicit; a future change to `exaudfclient`'s declared features would silently shrink coverage measurement |

### Consequences

The coverage job builds and measures correctly regardless of cache state, mirroring the existing CLAUDE.md rule that every `test-udfs/*` fixture a job's tests `dlopen` must be wired into that job's explicit build step. The measured feature set is now a declared, reviewable line in CI config instead of an inherited side effect of another crate's `Cargo.toml`.
