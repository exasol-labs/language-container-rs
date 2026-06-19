# Plan: add-connect-back-batch-execute

## Context

The `ExaConnection` trait exposes `execute` for single-statement DML, but no prepared-statement batch path: a UDF that needs to apply many parameterised DML rows (a bulk DELETE/UPDATE/INSERT) must issue one `execute` call per row, paying a round-trip and re-parsing the SQL each time. exarrow-rs 0.12.8 ships `Connection::prepare` / `Connection::execute_batch_update` / `Connection::close_prepared`, so the wire-level capability already exists — the gap is entirely in lc-rs.

This plan closes the lc-rs gaps so any UDF client can prepare a statement once and apply N parameter rows in a single batch call:

1. `execute_batch` method added to the `ExaConnection` trait (SDK, with a default returning `UdfError::Unimplemented`)
2. `RuntimeExaConnection::execute_batch` implemented in the runtime (prepare → map Values → batch update → close)
3. `Value`→`exarrow_rs::Parameter` mapping function added in the runtime
4. exarrow-rs version pin bumped from `0.12.7` to `^0.12.8`
5. Workspace version bumped from `0.12.1` to `0.13.0` and released to crates.io via tag-triggered CI

## Features

| Domain | Feature | Status | Spec delta |
|--------|---------|--------|------------|
| sdk | connect-back | CHANGED | `sdk/connect-back/spec.md` |
| runtime | connect-back | CHANGED | `runtime/connect-back/spec.md` |
| workspace | version | NEW | `workspace/version/spec.md` |

## Design

### Goals

- Expose a single `execute_batch(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<u64, UdfError>` method on the `ExaConnection` trait that lets any UDF author issue a parameterised batch DML call in one round-trip.
- Ship as `exasol-udf-sdk 0.13.0` on crates.io so downstream clients can pin the versioned crate.

### Non-Goals

- No exarrow-rs changes — the batch wire API is already present in 0.12.8.
- No binary-parameter support (`Value::Binary`) in this release — see the parameter-coverage ADR below; binary values can be hex-encoded into a `Value::String` by the caller, and a dedicated variant is added only when a concrete client needs raw-byte binding that hex cannot satisfy.
- No new integration test UDF — the existing connect-back-insert UDF is sufficient for manual smoke-testing; a dedicated batch-execute integration test UDF is deferred.
- No changes to the WebSocket transport feature.

### Architecture

```
UDF code (Box<dyn ExaConnection>)
  └── execute_batch(sql, rows: &[Vec<Value>])
        │
        ▼ (runtime, exa-udf-runtime)
  value_to_parameter(Value) → exarrow_rs::Parameter
        │
        ▼
  connect_back_rt().block_on(async {
    let stmt = conn.prepare(sql).await?;
    let param_rows = map_rows(rows)?;           // Vec<Vec<Parameter>>
    let count = conn.execute_batch_update(&stmt, &param_rows).await?;
    conn.close_prepared(stmt).await?;
    Ok(count)
  })
  (wrapped in catch_unwind)
```

### Parameter type coverage (binary values)

The SDK `Value` enum has no binary variant, while exarrow-rs `Parameter` does (`Parameter::Binary(Vec<u8>)`). The question is whether `execute_batch` needs a binary parameter path now, or whether the existing `Value::String` is sufficient for clients that must bind byte-oriented column types (e.g. an Exasol `HASHTYPE`, bound via `CAST(? AS HASHTYPE(n BYTE))`).

**Finding:** A client can bind binary data as a hex-encoded `Value::String` today. Inspecting exarrow-rs `parameter_to_json` (src/query/prepared.rs): `Parameter::String(s)` serialises to a JSON string, and `Parameter::Binary(b)` serialises to the *same* JSON string via `hex::encode(b)`. Both therefore produce identical wire bytes for hex data, and Exasol's prepared-statement protocol accepts a string value where a byte-typed column cast expects a hex literal (HASHTYPE columns accept hex-string literals). So `Value::String` carrying hex covers the binary case with no enum change.

**Decision: do not add `Value::Binary` in this release.** The five mapped variants (String, Integer, Float, Boolean, Null) plus hex-in-String cover the common batch-DML cases. A caller that needs raw-byte binding hex-encodes its bytes into `Value::String`.

**Upgrade path / risk flag:** This rests on protocol inspection and documentation, not a live test. If a future client hits a column type that genuinely rejects a hex-string parameter (a live smoke-test against a real Exasol instance shows rejection), add `Value::Binary(Vec<u8>)` to the SDK `Value` enum mapped to `Parameter::Binary`, update the `sdk/udf-sdk` spec, and that client passes raw bytes instead of hex.

### Trade-offs

| Decision | Alternative | Rationale |
|----------|-------------|-----------|
| Default `Unimplemented` on the trait, not a required method | Required method | Existing mock impls in unit tests keep compiling without changes |
| Return `Unimplemented` for Numeric/Date/Timestamp parameter binding | Silently render to string | Wrong literals are worse than clear errors; the common batch-DML cases bind String/Integer/Float/Boolean/Null |
| Empty-rows fast-path returns `Ok(0)` without server round-trip | Allow `execute_batch_update(&stmt, &[])` | Avoids an unnecessary prepare/close cycle for an empty batch |
| `close_prepared` errors are logged but do not replace execution errors | Propagate close error | Statement close failure is a resource-leak concern, not a data-integrity concern; the original error is more actionable |

### Key Interfaces

```rust
// sdk/connect-back — ExaConnection trait addition
fn execute_batch(
    &mut self,
    sql: &str,
    rows: &[Vec<Value>],
) -> Result<u64, UdfError> {
    Err(UdfError::Unimplemented("execute_batch not supported on this connection".into()))
}

// runtime — value_to_parameter helper (private)
fn value_to_parameter(v: &Value) -> Result<exarrow_rs::query::statement::Parameter, UdfError>;
```

## Tasks

### Group A — Dependency and version prep (no code logic, parallelisable with nothing blocked)

- [ ] A.1 Bump `exarrow-rs` in `[workspace.dependencies]` from `"0.12.7"` to `"^0.12.8"` in `Cargo.toml`
- [ ] A.2 Bump `[workspace.package].version` from `"0.12.1"` to `"0.13.0"` and update the `exasol-udf-sdk` workspace-dep `version` field to `"0.13.0"` in `Cargo.toml`
- [ ] A.3 Run `cargo check` to regenerate `Cargo.lock` with the new versions; commit `Cargo.toml` + `Cargo.lock` together

### Group B — SDK trait surface (depends on nothing; unblocks Group C)

- [ ] B.1 Add `execute_batch(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<u64, UdfError>` to the `ExaConnection` trait in `crates/exasol-udf-sdk/src/connect_back.rs` with the default body returning `Err(UdfError::Unimplemented("execute_batch not supported on this connection".into()))`
- [ ] B.2 Add a unit test `execute_batch_default_returns_unimplemented` in `crates/exasol-udf-sdk/tests/connect_back.rs` asserting that a minimal mock implementing only `query_arrow` and `execute` returns `UdfError::Unimplemented` from `execute_batch`
- [ ] B.3 Run `cargo test -p exasol-udf-sdk --features connect-back` and confirm all tests pass

### Group C — Runtime implementation (depends on A.1 + B.1) [expert]

- [ ] C.1 Add `fn value_to_parameter(v: &Value) -> Result<Parameter, UdfError>` in `crates/exa-udf-runtime/src/connect_back.rs` mapping the six `Value` variants: `Null→Null`, `Bool→Boolean`, `Int32→Integer(i as i64)`, `Int64→Integer`, `Double→Float`, `String→String`; return `Err(UdfError::Unimplemented(...))` for `Numeric`, `Date`, `Timestamp`
- [ ] C.2 Implement `ExaConnection::execute_batch` on `RuntimeExaConnection` in `crates/exa-udf-runtime/src/connect_back.rs` following the prepare→map→execute_batch_update→close_prepared lifecycle inside `connect_back_rt().block_on(async { … })` wrapped in `std::panic::catch_unwind(AssertUnwindSafe(…))`, mapping `QueryError` to `UdfError::ConnectBack`, catching panics into `UdfError::ConnectBack("panic in execute_batch: …")`, short-circuiting on empty `rows` with `Ok(0)`, and clamping negative counts to `0u64`
- [ ] C.3 Add a unit test `execute_batch_value_mapping_roundtrip` in `crates/exa-udf-runtime/src/connect_back.rs` (or a tests module) asserting `value_to_parameter` for each supported variant and the three unsupported variants that return `Unimplemented`
- [ ] C.4 Run `cargo test -p exa-udf-runtime --features connect-back` and confirm all tests pass

### Group D — Verification and release (depends on A + B + C)

- [ ] D.1 Run the full workspace test suite: `cargo test` — confirm 0 failures
- [ ] D.2 Run `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --check` — confirm 0 warnings and no format diffs
- [ ] D.3 Open a PR with all changes from Groups A–C; confirm CI passes (build + lint + unit tests; IT matrix skipped for tag-only trigger)
- [ ] D.4 After PR is merged to `main`, push the release tag: `ghbrk git push origin v0.13.0` — this triggers the CI `release` job which publishes `exasol-udf-sdk 0.13.0`, `exasol-udf-macros 0.13.0`, and `cargo-exasol-udf 0.13.0` to crates.io in dependency order **[IRREVERSIBLE once crates.io indexes the version]**

## Parallelization

| Group | Tasks | Depends on |
|-------|-------|------------|
| A | A.1, A.2, A.3 | — (A.3 depends on A.1 + A.2, but A.1 and A.2 are independent of each other) |
| B | B.1, B.2, B.3 | — (B.1 and B.2 can start immediately; B.3 depends on B.1 + B.2) |
| C | C.1, C.2, C.3, C.4 | A.1 (for exarrow-rs 0.12.8 APIs), B.1 (for trait method) |
| D | D.1, D.2, D.3, D.4 | A + B + C complete |

Groups A and B can run concurrently. Group C can start once A.1 and B.1 are done (the dep bump makes the new exarrow-rs API available; the trait addition is what C.2 implements). Group D is strictly sequential after all code work is merged.

## Verification

### Scenario Coverage

| Scenario | Test type | File | Test name |
|----------|-----------|------|-----------|
| sdk/connect-back — ExaConnection execute_batch default returns Unimplemented on a mock | unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | `execute_batch_default_returns_unimplemented` |
| sdk/connect-back — ExaConnection execute_batch signature has no exarrow-rs type | compile-time | `crates/exasol-udf-sdk/tests/connect_back.rs` | `execute_batch_default_returns_unimplemented` (same test; compilation verifies signature) |
| runtime/connect-back — RuntimeExaConnection implements execute_batch via prepare-batch-close lifecycle | unit | `crates/exa-udf-runtime/src/connect_back.rs` (or `tests/`) | `execute_batch_value_mapping_roundtrip` |
| runtime/connect-back — Value to Parameter mapping for execute_batch | unit | `crates/exa-udf-runtime/src/connect_back.rs` (or `tests/`) | `execute_batch_value_mapping_roundtrip` |
| workspace/version — exarrow-rs dependency is pinned to 0.12.8 | build | `Cargo.lock` inspection / `cargo check` | (no separate test; verified by `cargo check` resolving ≥ 0.12.8) |
| workspace/version — Workspace version is bumped to 0.13.0 | build | `Cargo.toml` | (no test; verified by CI tag-version-match step) |

### Manual Testing

| Feature | Command | Expected output |
|---------|---------|-----------------|
| sdk/connect-back | `cargo test -p exasol-udf-sdk --features connect-back` | All tests pass including `execute_batch_default_returns_unimplemented` |
| runtime/connect-back | `cargo test -p exa-udf-runtime --features connect-back` | All tests pass including `execute_batch_value_mapping_roundtrip` |
| Workspace build | `cargo build --release` | Exit 0; no exarrow-rs version errors |
| Full test suite | `cargo test` | 0 failures |
| Version tag | `git tag v0.13.0 && ghbrk git push origin v0.13.0` | CI release job triggers; crates.io shows `exasol-udf-sdk 0.13.0` after CI completes |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
| SDK feature-gated test | `cargo test -p exasol-udf-sdk --features connect-back` | Pass |
| Runtime connect-back test | `cargo test -p exa-udf-runtime --features connect-back` | Pass |
| Version consistency | `grep '^version' Cargo.toml` | `version = "0.13.0"` |
| exarrow-rs pin | `grep exarrow-rs Cargo.toml` | `version = "^0.12.8"` |
