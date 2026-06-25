# Plan: fix-abi-feature-safety

## Summary

Fix two coupled ABI/safety defects in the SDK and remove all Arrow C Data Interface
scope (the earlier `add-arrow-cdata-fast-path` plan — benchmarking showed Arrow IPC is
only 2–9% of `emit_batch` and the C Data Interface optimisation is not worth it):

- **#26** — `ExaConnection::query_arrow()` returns `Vec<arrow::RecordBatch>`, an
  unsafe cross-`.so` `TypeId` footgun (silent wrong values). Remove it; make the
  FFI-safe `query_for_each` (`Vec<Value>`) the required streaming method. No Arrow
  accessor replaces it — `Vec<Value>` is already the safe path.
- **#31** — `UdfContext`'s `#[cfg(feature=...)]`-gated trait methods shift the
  trait-object vtable when a UDF `.so` is built with a different feature set than the
  host SLC, so `emit_batch` silently dispatches to `cluster_ip` and emits 0 rows.
  Make the trait feature-independent (all methods always declared) and bump the ABI
  version so a stale `.so` is rejected loudly.

The two are coupled: removing `query_arrow` makes `ExaConnection` arrow-free, which
lets the connect-back types compile unconditionally — the prerequisite for an
always-declared (feature-independent) `UdfContext`.

## Design

### Context

The `.so`↔host call uses a stable `#[repr(C)] ExaUdfVTable` (`crates/exasol-udf-sdk/src/abi.rs`),
but the call **context** crosses as a `&mut dyn UdfContext` Rust trait object inside the
run shim. That trait-object vtable is ordered by method declaration, and the ABI
fingerprint (`SDK_VERSION:RUSTC_HASH`) does not encode the cargo feature set — so a
feature mismatch silently misroutes calls (issue #31). Separately, `ExaConnection::query_arrow`
hands a `repr(Rust)` `RecordBatch` across that boundary, where `TypeId`/vtables differ
between the two statically-linked `arrow` copies (issue #26).

- **Goals** — Eliminate the #26 footgun; make the `UdfContext` vtable identical across
  all feature combinations (#31); fail loudly on a stale `.so`.
- **Non-Goals** — Any Arrow C Data Interface work (`arrow-ffi`, `query_arrow_ffi`,
  `emit_batch_ffi`); changing the wire protocol or the 4 MB `MT_EMIT` flush; changing
  the `Value` API or the IPC `emit_batch` behaviour.

### Decision

1. **#26**: drop `query_arrow` from `ExaConnection`; `query_for_each(sql, FnMut(Vec<Value>))`
   becomes required; `query` keeps its default delegating to it. `ExaConnection` becomes
   arrow-free.
2. **#31**: remove every `#[cfg(feature=...)]` from `UdfContext` **method declarations**
   — `cluster_ip`/`connection`/`connect_back`/`emit_record_batch_ipc` always declared
   with `Unimplemented` defaults → feature-independent vtable. Enabled by (1): the
   `connect_back` module and `ConnectionObject`/`ExaConnection` now compile
   unconditionally (no SDK `connect-back` feature). `emit_record_batch_ipc(&[u8])` names
   no arrow type, so it is always declarable. The `emit-arrow` feature is narrowed to
   gate only `dep:arrow` + the `RecordBatch`-taking `EmitBatch` ext-trait. Bump
   `EXA_UDF_ABI_VERSION` 4 → 5.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Always-declared trait methods | `UdfContext` (context.rs) | Feature-independent `dyn` vtable layout (#31) |
| Unconditional module compile | `connect_back` (lib.rs) | Lets `connection`/`connect_back` be always-declared |
| `Vec<Value>` is the only cross-boundary row type | `ExaConnection` | No `repr(Rust)` Arrow across `.so` (#26) |
| Feature gates on deps/ext-traits only, never trait methods | SDK `emit-arrow` | Vtable stays stable; `arrow` dep still optional |
| ABI version bump as the tripwire | `abi.rs` + loader | Stale layout → clean `AbiMismatch`, not silent UB |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Remove `query_arrow`, no Arrow replacement | Keep + `query_arrow_ffi` (C Data Interface) | Benchmark: Arrow isn't the bottleneck; `Vec<Value>` is already safe and ergonomic. Drop the whole C Data Interface scope |
| Feature-independent trait (always declare) | Encode features in the fingerprint (detect-only) | Structural fix eliminates the skew class entirely; detect-only still can't interoperate. (Fingerprint-feature encoding optional follow-up) |
| Remove SDK `connect-back` feature | Keep it vestigial | After #26 the types are arrow-free; gating only adds the vtable-skew hazard back. Runtime keeps its own `connect-back` for heavy deps |
| Bump ABI version 4 → 5 | Leave it | Old `.so` built against the feature-dependent layout must fail loudly, not misdispatch |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/connect-back | CHANGED | `specs/_plans/fix-abi-feature-safety/sdk/connect-back/spec.md` |
| sdk/udf-sdk | CHANGED | `specs/_plans/fix-abi-feature-safety/sdk/udf-sdk/spec.md` |
| runtime/connect-back-query | CHANGED | `specs/_plans/fix-abi-feature-safety/runtime/connect-back-query/spec.md` |

## Dependencies

- None added or removed. The `arrow` dependency stays optional under `emit-arrow`
  (host runtime keeps it). The SDK `connect-back` feature is removed.

## Implementation Tasks

1. **SDK connect-back surface (sdk/connect-back) — #26**
   1.1 Remove `query_arrow` from `ExaConnection` (`crates/exasol-udf-sdk/src/connect_back.rs`); make `query_for_each` required; keep `query` defaulting to it; keep `execute`/`execute_batch`/`begin`/`commit`/`rollback` with their defaults.
   1.2 Un-gate the `connect_back` module and `ConnectionObject`/`ExaConnection` re-exports in `crates/exasol-udf-sdk/src/lib.rs`; remove the SDK `connect-back` cargo feature from `crates/exasol-udf-sdk/Cargo.toml`.
   1.3 Update mocks in `crates/exasol-udf-sdk/tests/connect_back.rs` and `crates/exa-udf-runtime/tests/connect_back.rs` to implement `query_for_each` instead of `query_arrow`; drop `query_arrow`-specific assertions.

2. **SDK UdfContext vtable (sdk/udf-sdk) — #31** [expert]
   2.1 Remove `#[cfg(feature=...)]` from the `UdfContext` method declarations (`cluster_ip`, `connection`, `connect_back`, `emit_record_batch_ipc`) in `crates/exasol-udf-sdk/src/context.rs`; all always-declared with `Unimplemented` defaults.
   2.2 Narrow the SDK `emit-arrow` feature to gate only `dep:arrow` + the `EmitBatch` ext-trait (which names `RecordBatch`); `emit_record_batch_ipc(&[u8])` is always declared.
   2.3 Bump `EXA_UDF_ABI_VERSION` 4 → 5 in `crates/exasol-udf-sdk/src/abi.rs`; leave the `#[repr(C)] ExaUdfVTable` field order unchanged.

3. **Runtime ripple**
   3.1 Update `crates/exa-udf-runtime/Cargo.toml`: its `connect-back` feature MUST stop referencing the removed `exasol-udf-sdk/connect-back`; keep it gating exarrow-rs/tokio/rustls + the `RuntimeExaConnection` impl.
   3.2 `crates/exa-udf-runtime/src/connect_back.rs` (runtime/connect-back-query): implement `query_for_each` directly (stream batches → `record_batch_to_rows` → callback → drop); remove the `query_arrow` impl.
   3.3 Audit `test-udfs/*/Cargo.toml` (and `benches/emit-bench-udf` on the bench branch later) for `exasol-udf-sdk/connect-back` feature refs and drop them (the feature no longer exists); the connect-back UDFs still compile since the types are unconditional.

4. **Version & verification**
   4.1 Bump `[workspace.package].version` + the pinned `exasol-udf-sdk` `[workspace.dependencies]` entry; regenerate `Cargo.lock`.
   4.2 Add/adjust tests per Verification below.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.x, 2.x (SDK: connect-back + UdfContext — independent files) |
| Group B | 3.x (runtime + manifests, after A) |
| Group C | 4.x (version, tests, after B) |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Method | `ExaConnection::query_arrow` (`crates/exasol-udf-sdk/src/connect_back.rs`) and its host impl (`crates/exa-udf-runtime/src/connect_back.rs`) | Unsafe cross-boundary `Vec<RecordBatch>` (#26); replaced by required `query_for_each` |
| Cargo feature | SDK `connect-back` feature (`crates/exasol-udf-sdk/Cargo.toml`) + `#[cfg(feature="connect-back")]` gates in context.rs/lib.rs | Vestigial after types go unconditional; the gate is the #31 hazard |
| Test code | `query_arrow` impls/asserts in the two `connect_back.rs` test files | Mocks implement `query_for_each` now |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| sdk/connect-back: ExaConnection trait is arrow-free and always compiled | Integration | `crates/exasol-udf-sdk/tests/connect_back.rs` | `exaconnection_arrow_free_no_feature_gate` |
| sdk/connect-back: query_arrow is removed from the cross-boundary trait surface | Integration | `crates/exasol-udf-sdk/tests/connect_back.rs` | `mock_implements_query_for_each_not_query_arrow` |
| sdk/connect-back: ConnectionObject is a public connect-back SDK type | Unit | `crates/exasol-udf-sdk/src/connect_back.rs` (`#[cfg(test)]`) | `connection_object_fields_public_unconditional` |
| sdk/udf-sdk: UdfContext vtable layout is feature-independent | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `emit_arrow_only_udf_emit_batch_dispatches_correctly` |
| sdk/udf-sdk: emit-arrow gates only the arrow dependency and the RecordBatch ext-trait | Integration | `crates/exasol-udf-sdk/tests/feature_gate.rs` | `emit_record_batch_ipc_present_without_emit_arrow` |
| sdk/udf-sdk: ABI version is bumped so a stale .so is rejected, not misdispatched | Unit | `crates/exa-udf-runtime/src/loader.rs` (`#[cfg(test)]`) | `abi_version_5_rejects_v4_so` |
| runtime/connect-back-query: RuntimeExaConnection streams query results as Value rows | Integration | `crates/exa-udf-runtime/tests/connect_back.rs` | `query_for_each_streams_value_rows` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-sdk (#31) | Build a UDF that calls `emit_batch` with the SDK default features only (no connect-back), register against the standard SLC, run a SET query via the `benches/` harness with the workaround removed | Emits all N rows (not 0) — the #31 repro now passes |
| sdk/udf-sdk (ABI) | Load a `.so` built against ABI v4 into the v5 host | Loader returns `AbiMismatch`, not silent 0-row emit |
| sdk/connect-back (#26) | `cargo build -p exasol-udf-sdk` (default features) | Exit 0; `connect_back` module + `ExaConnection` present with no `connect-back` feature and no `query_arrow` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build (default) | `cargo build --release` | Exit 0 |
| Build (all features) | `cargo build --release --all-features` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures (fails, not skips, if Docker DB unavailable) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
