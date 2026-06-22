# Plan: remove-sdk-dead-public-api

## Summary

Remove speculative, never-implemented public API from the `exasol-udf-sdk` crate (`column_count` alias, `column_name`/`column_type`/`column_index`/`reset` stubs, and `TryFrom<f64> for Decimal`), make one cold-path return-type simplification in `exaudfclient`, and bump the workspace version to 0.15.0 to signal the breaking change.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| `sdk/udf-sdk` | CHANGED | `specs/_plans/remove-sdk-dead-public-api/sdk/udf-sdk/spec.md` |
| `workspace/version` | CHANGED | `specs/_plans/remove-sdk-dead-public-api/workspace/version/spec.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Trait method | `crates/exasol-udf-sdk/src/context.rs:104-106` — `column_count()` default | Alias for `num_columns()`; adds noise, no caller exists |
| Trait method | `crates/exasol-udf-sdk/src/context.rs:108-112` — `column_name()` | Returns `Unimplemented` only; zero callers; never implemented in the host |
| Trait method | `crates/exasol-udf-sdk/src/context.rs:114-118` — `column_type()` | Returns `Unimplemented` only; zero callers; never implemented in the host |
| Trait method | `crates/exasol-udf-sdk/src/context.rs:120-124` — `column_index()` | Returns `Unimplemented` only; zero callers; never implemented in the host |
| Trait method | `crates/exasol-udf-sdk/src/context.rs:126-129` — `reset()` | Returns `Unimplemented` only; zero callers; never implemented in the host |
| Impl block | `crates/exasol-udf-sdk/src/value.rs:39-50` — `impl TryFrom<f64> for Decimal` | Wire/runtime path uses only `TryFrom<&str>`; f64→Decimal is lossy and never called |
| Unit test | `crates/exasol-udf-sdk/src/value.rs:133-139` — `from_f64` / NaN assertions within `decimal_from_str_and_f64_roundtrip` | Tests the removed `TryFrom<f64>` conversion |
| Doc line | `docs/writing-a-udf.md:185` — `ctx.column_type(col)` reference | References a removed method |

## Implementation Tasks

1. [ ] 1.1 Remove `column_count`, `column_name`, `column_type`, `column_index`, and `reset` from `UdfContext` trait in `crates/exasol-udf-sdk/src/context.rs`
2. [ ] 1.2 Remove `impl TryFrom<f64> for Decimal` block from `crates/exasol-udf-sdk/src/value.rs` and delete the `from_f64`/NaN sub-assertions in `decimal_from_str_and_f64_roundtrip`; rename remaining test to `decimal_from_str_roundtrip`
3. [ ] 1.3 Change `usage()` in `crates/exaudfclient/src/main.rs` to return `&'static str` instead of `String`
4. [ ] 1.4 Update `docs/writing-a-udf.md` line 185 to remove the `ctx.column_type(col)` example sentence
5. [ ] 1.5 Bump `version` in `[workspace.package]` and `exasol-udf-sdk` pin in `[workspace.dependencies]` in root `Cargo.toml` from `0.14.0` to `0.15.0`, then run `cargo build` to regenerate `Cargo.lock`

## Parallelization

Tasks 1.1, 1.2, 1.3, and 1.4 touch different files and MAY run concurrently.
Task 1.5 MUST follow tasks 1.1 and 1.2 (need a clean build to confirm the version bump compiles).

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1, 1.2, 1.3, 1.4 |
| Group B | 1.5 |

Sequential dependencies:
- Group A → Group B (1.5 requires a clean compile after the removals)

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Value and ExaType cover the v1 column types | Unit | `crates/exasol-udf-sdk/src/value.rs` | `value_exatype_typed_variants` |
| Decimal is constructible from string without precision loss | Unit | `crates/exasol-udf-sdk/src/value.rs` | `decimal_from_str_roundtrip` |
| UdfContext exposes typed accessors and row iteration | Unit | `crates/exasol-udf-sdk/src/context.rs` | `bridge_typed_getters_return_typed_options` |
| UdfRun default single-call hooks return Unimplemented | Unit | `crates/exasol-udf-sdk/src/context.rs` | `default_hooks_unimplemented` |
| Workspace version is bumped to 0.15.0 for the dead-API removal release | Unit | `Cargo.toml` / `Cargo.lock` | n/a — verified by `cargo build` exit 0 and `grep version Cargo.toml` |
| Wrong argument count is rejected | Unit | `crates/exaudfclient/src/main.rs` | `too_few_args_returns_exit_code_1` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| `sdk/udf-sdk` removal compiles | `cargo build --release` | Exit 0, no errors about removed methods |
| `sdk/udf-sdk` tests pass | `cargo test -p exasol-udf-sdk` | 0 failures |
| `exaudfclient` usage change | `cargo test -p exaudfclient` | 0 failures |
| Version bump visible | `grep 'version = "0.15.0"' Cargo.toml` | Two matches (package + sdk pin) |
| Lint clean | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
