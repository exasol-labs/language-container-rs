# Plan: add-memory-limit-metadata

## Summary

Surface the per-UDF-instance resident-memory limit (`exascript_info.maximal_memory_limit`, bytes) from the handshake Info response onto `UdfMeta` and expose it to UDF authors via a new `UdfContext::memory_limit()` accessor, so a downstream UDF (e.g. a DataFusion scan) can size an in-process memory pool to the limit the database allotted.

## Design

This is a small enabling change that mirrors two existing, well-established idioms in the codebase; no new architecture is introduced.

### Context

The `localzmq+protobuf` wire protocol already carries the value — `exascript_info.maximal_memory_limit` (field 11, `required uint64`, bytes, per-UDF-instance, enforced by the DB via `setrlimit(RLIMIT_RSS)`). The C++/Python/Java/R SLCs already expose it (`memoryLimit()` / `meta.memory_limit`); the Rust SLC does not. The gap is purely in deserialization (`UdfMeta::from_pb` drops the field) and the SDK surface (`UdfContext` has no accessor). A downstream lakehouse-engine UDF needs the value to bound a DataFusion memory pool to the sandbox limit.

- **Goals** — Decode `maximal_memory_limit` onto `UdfMeta`; expose it to UDF code as bytes via `UdfContext`; keep all existing `UdfContext` impls compiling.
- **Non-Goals** — No enforcement of the limit inside the Rust SLC (the DB enforces it); no change to the proto schema; no rescaling/unit conversion; no new vtable/ABI surface (the accessor lives behind the existing host-side trait object, not across the `.so` boundary).

### Decision

Thread the value through the existing handshake → `UdfMeta` → `HostContextBridge` → `UdfContext` path, mirroring the `node_count` field (decode) and the `cluster_ip()` accessor shape (trait method with a default), but without the `connect-back` feature gate since this is plain metadata.

#### Architecture

```
exascript_info.maximal_memory_limit (u64, bytes, field 11)
        │  UdfMeta::from_pb (mirror node_count)
        ▼
UdfMeta.maximal_memory_limit: u64
        │  dispatch already holds &UdfMeta; pass into bridge
        ▼
HostContextBridge.memory_limit (u64)
        │  overrides defaulted trait method
        ▼
UdfContext::memory_limit(&self) -> u64   (default impl returns 0)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Mirror `node_count` decode | `UdfMeta` field + `from_pb` | The field is the exact same shape (scalar from `exascript_info`); copy the proven idiom |
| Defaulted trait accessor returning a sentinel | `UdfContext::memory_limit` | Keeps every existing `UdfContext` impl (incl. test doubles, `SingleCallContext`) compiling without edits |
| Override in host bridge | `HostContextBridge` | Only the real host context has the live `UdfMeta`; everything else gets the `0` default |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Return `u64` bytes, no unit conversion | Return a typed `ByteSize`/`Option<u64>` | The proto unit is bytes and the consumer (memory pool sizing) wants raw bytes; matching the proto avoids lossy conversion and an extra type |
| `0` sentinel for "no limit reported" | `Option<u64>` accessor; panic on absent | The proto field is `required` so it is effectively always present (prost defaults to `0`); `0` is a natural "unbounded/unknown" sentinel and keeps the accessor signature trivial for the common path |
| Defaulted (not required) trait method | Required method on `UdfContext` | A required method would force edits to `SingleCallContext` and every test double; a default returning `0` is the same backward-compat idiom the SDK already uses |
| No `connect-back` feature gate | Gate like `cluster_ip()` | The limit is handshake metadata available unconditionally; gating it would needlessly hide it from non-connect-back UDFs |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| protocol/wire-protocol | CHANGED | `protocol/wire-protocol/spec.md` |
| sdk/udf-sdk | CHANGED | `sdk/udf-sdk/spec.md` |

## Implementation Tasks

1. **wire-protocol decode**
   1. Add `pub maximal_memory_limit: u64` to `UdfMeta` in `crates/exa-zmq-protocol/src/meta.rs` (next to `node_count`), with a doc comment noting "bytes, per-UDF-instance resident-memory limit".
   2. Extract it in `UdfMeta::from_pb`: `maximal_memory_limit: info.maximal_memory_limit` (mirror the `node_count: info.node_count` line). Update any other `UdfMeta { .. }` literals that don't use `..` to set the new field.
   3. Add a unit test in `meta.rs` asserting `from_pb` carries a non-zero `maximal_memory_limit` from the `exascript_info`, and that an info with the field at its default surfaces `0`.

2. **sdk accessor**
   1. Add `fn memory_limit(&self) -> u64 { 0 }` as a defaulted method on the `UdfContext` trait in `crates/exasol-udf-sdk/src/context.rs`, with a doc comment (bytes; `0` = no limit reported; not connect-back gated).
   2. Add a unit test in `context.rs` asserting the default returns `0` for a context that does not override it.

3. **runtime wiring**
   1. Add a `memory_limit: u64` field to `HostContextBridge` in `crates/exa-udf-runtime/src/rowset.rs` and set it from `&UdfMeta` where the bridge is constructed (in `crates/exa-udf-runtime/src/dispatch.rs`, which already holds `&UdfMeta`).
   2. Implement `fn memory_limit(&self) -> u64 { self.memory_limit }` in `impl UdfContext for HostContextBridge`.
   3. Add a unit test (in `rowset.rs`) asserting the bridge returns the byte value it was constructed with.

4. **verification** — run the full checklist (build, test, clippy, fmt) per `specs/mission.md § Commands`.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 (wire-protocol decode), Task 2 (sdk accessor) |
| Group B | Task 3 (runtime wiring) |

Sequential dependencies:
- Group A → Group B (the bridge wiring in Task 3 depends on both the `UdfMeta` field from Task 1 and the trait method from Task 2).
- Task 4 runs last.

## Dead Code Removal

None. This is purely additive; no existing code becomes obsolete.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Memory limit is surfaced from the handshake info response | Unit | `crates/exa-zmq-protocol/src/meta.rs` (`#[cfg(test)] mod tests`) | `from_pb_carries_maximal_memory_limit` |
| UdfContext exposes the per-instance memory limit | Unit | `crates/exasol-udf-sdk/src/context.rs` (`#[cfg(test)] mod tests`) + `crates/exa-udf-runtime/src/rowset.rs` (bridge override) | `default_memory_limit_is_zero` / `bridge_returns_memory_limit` |

Both scenarios cover pure in-process value plumbing (proto struct → Rust struct → trait accessor) with no I/O, so unit tests are the correct and sufficient form of external proof; no live-DB integration test is warranted for a field carried verbatim through deserialization.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| protocol/wire-protocol | `cargo test -p exa-zmq-protocol from_pb_carries_maximal_memory_limit` | 1 passed; `UdfMeta::maximal_memory_limit` equals the proto value |
| sdk/udf-sdk | `cargo test -p exasol-udf-sdk default_memory_limit_is_zero` | 1 passed; default accessor returns `0` |
| sdk/udf-sdk (runtime override) | `cargo test -p exa-udf-runtime bridge_returns_memory_limit` | 1 passed; bridge returns the constructed byte value |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
