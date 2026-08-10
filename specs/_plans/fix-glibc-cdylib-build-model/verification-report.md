# Verification Report: fix-glibc-cdylib-build-model

## Verdict

| Result | Details |
|--------|---------|
| **PASS** (host-side) | All host checks green; `cargo exasol-udf build` produces a loadable glibc cdylib and the build subcommand is now gated by un-ignored tests. Integration + e2e run at the implement-pr record gate (post version-bump, rebuilt artifacts). |
| Code review | 2 findings — 2 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ (CLI build) / gated (it) |
| Manual Tests | ✓ (via automated equivalent) |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit + integration-in-crate (`cargo test`) | all workspace | all passed, 0 failed | 2 (pre-existing, unrelated) |
| `cargo-exasol-udf` build suite (`--test build`) | 4 | 4 | 0 |
| Live DB (`it`, `--features integration`) | — | — | gated to implement-pr step 6 |

The `cargo-exasol-udf` build suite is the plan's anti-regression gate. Before this plan its three build tests were `#[ignore]`d, hiding a fully-broken subcommand. Now:

```
running 4 tests
test build_fails_on_missing_cargo_toml ... ok
test build_fails_when_artifact_missing_at_expected_path ... ok
test build_produces_host_cdylib ... ok
test build_honors_target_override ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

`build_fails_when_artifact_missing_at_expected_path` was added while fixing review finding 1 — it reproduces the swallowed-error scenario (a `[lib] name` override making cargo succeed while `lib<crate>.so` never appears).

### Manual Tests

| Test | Result |
|------|--------|
| `cargo exasol-udf new` + `build` → prints `target/release/lib<crate>.so`, file exists, exports `__exa_udf_entry_*` | ✓ (proven by `build_produces_host_cdylib`, which scaffolds via `new`, patches the scaffold to the local workspace SDK, builds, and asserts the entry symbol) |

A live `cargo exasol-udf new /tmp/demo && cargo exasol-udf build` run against crates.io depends on the SDK version being published (the release step); the automated test proves the same flow against the local SDK deterministically.

## Tool Evidence

### Linter

```
$ cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile — exit 0, no warnings
```

### Formatter

```
$ cargo fmt --check
exit 0 — no changes
```

### Build

```
$ cargo build --release
Finished `release` profile [optimized] — exit 0
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| tools | cargo-exaudf | build produces a loadable host cdylib | `crates/cargo-exasol-udf/tests/build.rs` | `build_produces_host_cdylib` | Pass |
| tools | cargo-exaudf | build honors an explicit `--target` override | `crates/cargo-exasol-udf/tests/build.rs` | `build_honors_target_override` | Pass |
| tools | cargo-exaudf | build verifies the artifact exports a named entry point | `crates/cargo-exasol-udf/tests/build.rs` | `build_produces_host_cdylib` | Pass |
| tools | cargo-exaudf | new scaffolds a buildable UDF crate | `crates/cargo-exasol-udf/tests/build.rs` | `build_produces_host_cdylib` | Pass |
| tools | cargo-exaudf | build errors when cargo succeeds but produces no artifact | `crates/cargo-exasol-udf/tests/build.rs` | `build_fails_when_artifact_missing_at_expected_path` | Pass |
| examples | test-udfs / -timestamps / -connect-back | glibc-cdylib compile + roundtrip behavior | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` | Gated to implement-pr step 6 |

The example-fixture scenarios changed only spec wording (dropped the musl triple); their glibc-cdylib compile + behavior is proven by the existing `it` roundtrip scenarios, which run at the record gate against a live Exasol Docker DB with artifacts rebuilt at the bumped version.

## Notes

- **Integration/e2e gating is intentional.** The version bump changes the ABI fingerprint (`SDK_VERSION:RUSTC_HASH`); the SLC tarball and all `test-udfs/*.so` fixtures must be rebuilt at the bumped version before the live-DB suite runs, so integration is deferred to the implement-pr record gate (step 6), not run here pre-bump.
- **`--target` missing-value path** returns `Err` (not a panic); not separately unit-tested — the plan's test list is the three build tests. Accepted by review.
- **`about.toml` musl pin** intentionally untouched — deferred to PR #79 per plan § Dead Code Removal.
- Two pre-existing `#[ignore]`d tests elsewhere in the workspace are unrelated to this plan and out of scope.
