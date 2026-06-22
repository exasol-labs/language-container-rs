# Verification Report: remove-sdk-dead-public-api

## Verdict

**PASS** (unit-level) — all automated checks green; breaking removals compile and
test clean; version bumped to 0.15.0. Full live-Docker E2E re-run as the landing
gate for the version bump (result appended below).

## Summary

| Check | Result |
|-------|--------|
| `cargo build` (debug) | ✅ exit 0 — all crates at v0.15.0 |
| `cargo test --workspace --exclude it` | ✅ 0 failures |
| `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| `cargo fmt --check` | ✅ no changes |
| `Cargo.lock` regenerated | ✅ `exasol-udf-sdk` 0.15.0 |
| Full E2E (`scripts/ci-it-local.sh`) | ✅ rc=0, 24/24 scenarios (gate re-run) |

## Changes

Net **−64/+14** lines across 6 files. Pure removal of dead/test-only public API,
one cosmetic launcher tweak, and the version bump.

- `crates/exasol-udf-sdk/src/context.rs` — removed `column_count`, `column_name`,
  `column_type`, `column_index`, `reset` from `UdfContext`; dropped the now-unused
  `ExaType` import.
- `crates/exasol-udf-sdk/src/value.rs` — removed `impl TryFrom<f64> for Decimal`;
  renamed `decimal_from_str_and_f64_roundtrip` → `decimal_from_str_roundtrip` and
  dropped the f64/NaN sub-assertions. `TryFrom<&str>` (the wire path) retained.
- `crates/exaudfclient/src/main.rs` — `usage()` returns `&'static str` (no alloc).
  No change to the `Exit` struct or the `std::process::exit(0)` lifecycle path.
- `docs/writing-a-udf.md` — removed the `ctx.column_type(col)` accessor claim.
- `Cargo.toml` / `Cargo.lock` — workspace version + SDK pin 0.14.0 → 0.15.0.

## Scenario Coverage

| Scenario | Test | Status |
|----------|------|--------|
| Value and ExaType cover the v1 column types | `value_exatype_typed_variants` | ✅ pass |
| Decimal is constructible from string | `decimal_from_str_roundtrip` | ✅ pass |
| UdfContext exposes typed accessors and row iteration | `bridge_typed_getters_return_typed_options` | ✅ pass |
| UdfRun default single-call hooks return Unimplemented | `default_hooks_unimplemented` | ✅ pass |
| Workspace version bumped to 0.15.0 | `cargo build` exit 0 + `grep` Cargo.toml | ✅ pass |
| Wrong argument count is rejected | `too_few_args_returns_exit_code_1` | ✅ pass |

## Code Review

No findings. All changes are deletions of never-called/test-only API, a cosmetic
return-type change, and a version bump. No dead code introduced, no obsolete tests
left behind, core trait methods (`num_columns`/`get`/`emit`/`next`) intact.

## Breaking Change Note

Removing public `UdfContext` methods and `Decimal: TryFrom<f64>` is API-breaking;
0.15.0 is the breaking bump under 0.x SemVer. On merge to `main`, ci.yml auto-tags
and releases v0.15.0. No in-repo callers existed (verified by grep); no test-udf
used the removed surface.
