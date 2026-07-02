# Plan: fix-single-call-handshake-metadata

## Summary

Thread the `exascript_info` handshake metadata into `SingleCallContext` so a virtual-schema adapter call (`SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`) sees the same live `node_count`/`node_id`/`session_id`/etc. the scalar/set `HostContextBridge` already surfaces, fixing GitHub issue #41 where these accessors silently returned neutral defaults (notably `node_count() == 0`, which makes the downstream `lakehouse-engine-rs` consumer plan cluster fan-out as single-node).

## Design

### Context

The DB delivers the `exascript_info` handshake once per session (`MT_META`). PR #40 (`add-udfcontext-handshake-metadata`) exposed those fields to UDF code through defaulted `UdfContext` accessors overridden by the streaming-path `HostContextBridge`. The single-call path was not touched: `run_single_call` receives `&UdfMeta` but binds it `_meta` (unused), and `SingleCallContext` overrides only `num_columns`/`debug_level`/`get`/`emit`/`next` (plus the connect-back trio), so every handshake accessor falls back to the trait default. For a `createVirtualSchema` adapter call this means `ctx.node_count()` always returns `0` on a cluster of any size, with no error — the value is dropped before the adapter sees it.

- **Goals** — give `SingleCallContext` full parity with `HostContextBridge` on the handshake accessors, reusing the existing `HandshakeMeta` struct and its `From<&UdfMeta>` impl; prove the fix end-to-end with a real `CREATE VIRTUAL SCHEMA` round-trip that asserts the adapter hook receives live (non-neutral) metadata.
- **Non-Goals** — no change to the `UdfContext` trait surface (the accessors already exist from PR #40); no new connect-back capability; no change to the streaming `HostContextBridge`; no change to the other single-call hooks (`default_output_columns`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec` take no `UdfContext` pointer, so they are unaffected).

### Decision

Follow the `HostContextBridge` pattern exactly. `SingleCallContext` gains a `handshake: HandshakeMeta` field, threaded through `SingleCallContext::new(...)`, and its `impl UdfContext` gains one-line passthrough overrides for every handshake accessor (numeric by value, `String`/`Option<String>` by `.clone()`), none feature-gated. `run_single_call` renames `_meta` → `meta`, builds `HandshakeMeta::from(meta)` once, and threads it (by `Clone`) through `invoke_hook` → `invoke_vs_adapter_call` → `SingleCallContext::new(...)`. `SingleCallContext` is constructed in exactly one place (`invoke_vs_adapter_call`), so no other call site is affected.

#### Architecture

```
MT_META (exascript_info)
      │ UdfMeta  ── run_single_call(meta) ── HandshakeMeta::from(meta)
      │                                              │  (Clone, threaded down)
      │                                              ▼
      │                       invoke_hook → invoke_vs_adapter_call
      │                                              │
      │                                              ▼
      │                        SingleCallContext { handshake, .. }
      │                                              │ overrides (parity w/ bridge)
      └──────────────────────────────────▶  &mut dyn UdfContext (adapter hook)
                                                     live handshake values
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Reuse `HandshakeMeta` + its `From<&UdfMeta>` | `SingleCallContext` field | Single owned snapshot type shared with the bridge; no parallel type |
| Host bridge override (per-accessor passthrough) | `impl UdfContext for SingleCallContext` | Mirrors `HostContextBridge` exactly so both paths behave identically |
| Owned return across the vtable | string/optional accessors | `String`/`Option<String>` cross the `.so` boundary safely; no borrow entanglement |
| Deliberate-error echo through the single-call error channel | `single-call-fixture` adapter shim | Surfaces live metadata to the IT client via the already-proven rc!=0 out-pointer path, without constructing a full valid adapter-protocol response |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Full parity: override all 13 handshake accessors on `SingleCallContext` | Minimal fix (`node_count`/`node_id` only) | Interview decision: any single-call consumer needing topology/session/instance metadata should get live values, not a partial fix that re-litigates later |
| Reuse `HandshakeMeta` and its `From` impl | New single-call-specific meta type | Struct + `From` already exist and are `Clone`; a parallel type is dead duplication |
| Add `handshake: HandshakeMeta` param to `SingleCallContext::new` (non-feature-gated) | Gate behind `connect-back` | Handshake metadata is plain DB context, not a connect-back capability (matches the bridge and the SDK spec) |
| IT adapter fixture echoes live metadata via the deliberate-error channel (CREATE VIRTUAL SCHEMA expected to fail with metadata in the error text) | Return a valid `createVirtualSchema` response encoding metadata in table/column identifiers, then query the created schema | The error channel is version-robust and directly asserts the exact bug; a valid schema-metadata response is brittle across DB versions and needs numeric-in-identifier parsing to read values back |
| Version bump = PATCH (0.20.0 → 0.20.1) | MINOR | No SDK trait surface change; this is a runtime-internal bug fix that wires up already-existing accessors |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| runtime/dispatch-single-call | CHANGED | `runtime/dispatch-single-call/spec.md` |

Note: `sdk/udf-sdk` already specifies the handshake accessors (added by PR #40) and its trait surface does not change, so it gets no delta. The `single-call-fixture` test crate and the new IT scenario are verification mechanics (per CLAUDE.md, test-harness mechanics are not spec content) and are captured under Verification below.

## Implementation Tasks

1. Add a `handshake: HandshakeMeta` field to `SingleCallContext<'a>` and thread it through `SingleCallContext::new(...)` (add the param before/around the existing `#[cfg(feature = "connect-back")] conn_requester`), storing it in the constructor (`crates/exa-udf-runtime/src/rowset.rs`).
2. Add the 13 handshake accessor overrides to `impl UdfContext for SingleCallContext<'_>` — `memory_limit`, `session_id`, `statement_id`, `node_id`, `node_count`, `vm_id` (numeric, return `self.handshake.<field>`) and `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user` (`.clone()`), each an exact copy of the `HostContextBridge` override, none feature-gated (`crates/exa-udf-runtime/src/rowset.rs`). [expert]
3. Thread the handshake through the single-call dispatcher: rename `_meta` → `meta` in `run_single_call`, build `let handshake = crate::rowset::HandshakeMeta::from(meta);`, and pass it (by `Clone`) through `invoke_hook` and `invoke_vs_adapter_call` into `SingleCallContext::new(...)`; update the three function signatures (`crates/exa-udf-runtime/src/single_call.rs`). [expert]
4. Fix the two existing in-crate `SingleCallContext::new(...)` unit-test call sites to pass the new `handshake` arg (`single_call_context_debug_level_returns_valid_level` and the connect-back test around `crates/exa-udf-runtime/src/rowset.rs:1623-1629`) — pass `HandshakeMeta::default()`.
5. Add a `single_call_context_returns_handshake_metadata` unit test mirroring `bridge_returns_handshake_metadata`: construct a `SingleCallContext` from a populated `HandshakeMeta` (present `current_user`, absent `current_schema`/`scope_user`) and assert every accessor returns the exact value, including `Some`/`None` for the optionals (`crates/exa-udf-runtime/src/rowset.rs` test module).
6. Extend the `single-call-fixture` adapter shim to read live metadata off the ctx pointer: in `virtual_schema_adapter_call`, restore `&mut dyn UdfContext` via the double-indirection ABI (`let ctx: &mut dyn UdfContext = unsafe { &mut **(_ctx as *mut &mut dyn UdfContext) };`, matching how `invoke_vs_adapter_call` builds `ctx_ptr` and how the run shim restores it), then return rc=1 writing an error string that embeds the live values (e.g. `HANDSHAKE_META node_count=<n> node_id=<n> session_id=<n> script_name=<s>`) into the `result` out-pointer so the runtime surfaces it. Keep `default_output_columns` and the vtable wiring unchanged (`test-udfs/single-call-fixture/src/lib.rs`). [expert]
7. Add a `single_call_adapter_surfaces_live_handshake_metadata` IT scenario in `crates/it/tests/db_roundtrip.rs`: register the fixture as a `RUST ADAPTER SCRIPT` (`%udf_object` framing), issue a real `CREATE VIRTUAL SCHEMA <n> USING <schema>.<adapter_script>` to trigger `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL` (`createVirtualSchema`), expect the statement to fail, and assert the surfaced error text contains `node_count=<n>` with `n != 0` (the neutral-default gate) plus a parseable `node_id`/`session_id` and the registered script name — mirroring the assertion style of `handshake_metadata_udf_emits_session_and_node`. Wire the scenario into `db_roundtrip_all_scenarios` and reuse the existing `SC_LIB` upload. [expert]
8. Bump `[workspace.package].version` 0.20.0 → 0.20.1 and the pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` to `0.20.1`, then regenerate and commit `Cargo.lock` (PATCH: runtime-internal bug fix, no SDK surface change) — per CLAUDE.md SemVer rule.

## Dependencies

- Relies on the `HandshakeMeta` struct and its `From<&UdfMeta>` impl added by PR #40 (`crates/exa-udf-runtime/src/rowset.rs`). No new external crates.
- CI: `.github/workflows/ci.yml` already lists `-p single-call-fixture` in the "Build UDF .so artifacts (release)" allowlist (line 66) and in the debug build step (line 163), so no CI wiring change is required — only the fixture's behavior changes, not its identity. (Verified.)

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 |
| Group B | Task 2, Task 3, Task 4 |
| Group C | Task 5, Task 6 |
| Group D | Task 7 |
| Group E | Task 8 |

Sequential dependencies:
- Group A (field + constructor) → Group B (accessors read the field; dispatcher + test sites pass the arg)
- Group B → Group C (unit test asserts the accessors; fixture exercises them)
- Group C → Group D (IT needs the fixture behavior)
- Group D → Group E (version bump last, after code + tests settle)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| (none) | — | The fix only wires up an already-received-but-discarded `_meta` param; nothing becomes obsolete |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Adapter single-call context surfaces live handshake metadata | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `single_call_context_returns_handshake_metadata` |
| Adapter single-call context surfaces live handshake metadata (end-to-end live values) | Integration | `crates/it/tests/db_roundtrip.rs` | `single_call_adapter_surfaces_live_handshake_metadata` |

Note: the accessor overrides are pure computation over in-memory state (no I/O), covered by the unit test; the live-value path through a real `createVirtualSchema` adapter call is covered by the integration scenario, which also confirms the DB actually populates `exascript_info` for the VS-adapter single-call path (the issue's "not yet verified" gap).

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| runtime/dispatch-single-call | Build & upload the `single-call-fixture` `.so`, register it as `CREATE OR REPLACE RUST ADAPTER SCRIPT sc_adapter AS %udf_object /buckets/.../libsingle_call_fixture.so; /` then run `CREATE VIRTUAL SCHEMA vs_hs USING sc_adapter;` against a local Exasol Docker DB | The statement fails with an adapter error whose text contains `node_count=<n>` with a non-zero `n` and the registered script name (proving live handshake metadata reached the adapter hook) |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Unit test | `cargo test` | 0 failures |
| Integration test | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
