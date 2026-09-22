# Plan: add-import-export-spec-hooks

## Summary

Makes `IMPORT INTO (...) FROM SCRIPT` and `EXPORT INTO SCRIPT` usable from a Rust UDF by wiring the `generate_sql_for_import_spec` / `generate_sql_for_export_spec` hooks end to end: macro annotation, decoded specification payload, and a live `UdfContext`. Closes the three naming and metadata parity gaps that ride the same SDK version bump.

## Design

### Context

The two spec-generation vtable slots and their dispatch arm exist, but nothing reaches them. The macro hardcodes both slots to `None`, the dispatcher passes the virtual-schema `json_arg` instead of the decoded specification message, and the slot signature carries no context pointer. A hook that generates a `SELECT` needs `script_schema()` to qualify the worker script, `node_count()` to size an export shard count, and `connection(name)` to validate a target system before returning SQL.

- **Goals**: an author annotates a function, receives the full specification, and reads live handshake metadata and CONNECTION credentials while building the SQL.
- **Non-Goals**: row-emitting UDFs, `GROUP BY` parallel loading, and direct-`SELECT` export UDFs, all already served by EMIT and connect-back.

### Decision

Both spec-generation slots adopt the `virtual_schema_adapter_call` slot shape, so one dispatcher path serves every context-taking single-call hook. The specification message is protobuf, which is not ABI-stable across the `.so` boundary, so one runtime module serializes it to JSON and that JSON is the author-facing payload.

#### Architecture

```
MT_CALL(SC_FN_GENERATE_SQL_FOR_IMPORT_SPEC, import_specification)
        │
        ▼
exa-zmq-protocol  HostEvent::SingleCall { fn_id, json_arg, import_spec, export_spec }
        │
        ▼
exa-udf-runtime   spec_json.rs ──── serialize ────▶ json_spec: String
        │                                               │
        │  SingleCallContext (handshake meta + MT_IMPORT credential channel)
        ▼                                               ▼
  vtable slot (ctx: *mut c_void, json_spec: *const c_char, result: *mut *mut c_char)
        │
        ▼
UDF .so   generated shim ──▶ #[exasol_udf(import_spec(f))] f(ctx, json_spec) -> SQL
                                                                  │
                                       optional import feature ──┴──▶ ImportSpec::from_json
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Double-indirected `&mut dyn UdfContext` behind `*mut c_void` | both spec slots | a fat trait-object pointer cannot cross a thin C ABI slot; the `run` and vs-adapter slots already use it |
| Single owning module for the wire shape | `crates/exa-udf-runtime/src/spec_json.rs` | one module decides the JSON shape, so no other module encodes that decision |
| Mechanical 1:1 proto mirror | the `json_spec` object and the `ImportSpec` / `ExportSpec` structs | the mapping is derivable from `zmqcontainer.proto` alone, so a proto change needs no translation table |
| Optional cargo feature for an added dependency | `import` and `export` on `exasol-udf-sdk` | a default UDF build keeps its dependency set, the shape `emit-arrow` already uses |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|-------------------------|-----------|
| Hooks receive `&str` JSON, with typed `ImportSpec` / `ExportSpec` parsing behind the non-default `import` and `export` features | a typed struct as the hook parameter, or one combined feature for both structs | a typed parameter adds `serde` to every UDF build and makes the vtable layout depend on a feature. One feature per statement names what the author is writing and carries no code for the other statement |
| Two SDK enums for the iteration axes, `InputType` and `OutputType` | reuse the protocol crate's single `IterType` | the author writes SCALAR, SET, RETURNS and EMITS in SQL, so the SDK names those shapes rather than the proto's `ExactlyOnce` and `Multiple` |
| `parameters` mirrors `key_value_pair` as an array | collapse to a JSON object map | the map form is lossy for duplicate keys and breaks the "derivable from the proto" property |
| `rows_in_group` is verified against a live database | unit tests only | the engine sets the field on every batch it sends (`../db/Engine/src/exscript/pluggable/zmqcontainer.cc:415`, recorded in `FINDINGS.md:258-262`), so the live scenario pins the group cardinality the database actually reports |
| One ABI bump `9 → 10` covers every change | separate parallel slots to preserve ABI 9 | the version is half the fingerprint, so any SDK bump already forces downstream rebuilds |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-sdk | CHANGED | `specs/_plans/add-import-export-spec-hooks/sdk/udf-sdk/spec.md` |
| sdk/udf-abi | CHANGED | `specs/_plans/add-import-export-spec-hooks/sdk/udf-abi/spec.md` |
| sdk/udf-macro | CHANGED | `specs/_plans/add-import-export-spec-hooks/sdk/udf-macro/spec.md` |
| protocol/single-call | CHANGED | `specs/_plans/add-import-export-spec-hooks/protocol/single-call/spec.md` |
| runtime/dispatch-single-call | CHANGED | `specs/_plans/add-import-export-spec-hooks/runtime/dispatch-single-call/spec.md` |
| runtime/rowset-codec | CHANGED | `specs/_plans/add-import-export-spec-hooks/runtime/rowset-codec/spec.md` |
| examples/test-udfs | CHANGED | `specs/_plans/add-import-export-spec-hooks/examples/test-udfs/spec.md` |

## Impact

`IMPORT INTO (...) FROM SCRIPT` and `EXPORT INTO SCRIPT` become available to Rust UDF authors.

Additive changes:

- The non-default `import` and `export` features give authors typed `ImportSpec` and `ExportSpec` parsing of the `json_spec` payload. A default build keeps its current dependency set.
- `UdfContext` gains the defaulted `input_type` and `output_type` accessors, so no existing implementation changes.

Breaking changes for downstream users:

- `EXA_UDF_ABI_VERSION` goes `9 → 10`. Every deployed `.so` must be rebuilt, and a stale one fails the loader check with `AbiMismatch`.
- `UdfContext::num_columns` is removed. `input_column_count` is the only name, so a hand-written `impl UdfContext` or call site, in this repository or downstream, renames the method or fails to compile.
- The release needs a minor `[workspace.package].version` bump, the matching `exasol-udf-sdk` pin in `[workspace.dependencies]`, and a regenerated `Cargo.lock` in the same PR.

## Dependencies

No new third-party crates enter the workspace. `crates/exa-udf-runtime` already depends on `serde_json`, and `[workspace.dependencies]` already pins both `serde` and `serde_json`.

The two new non-default features, `import` and `export`, each add `serde` and `serde_json` to `crates/exasol-udf-sdk` as optional dependencies, so a default UDF build takes neither. `serde` then reaches a published crate, which the `[workspace.dependencies]` comment scoping it to the benchmark suite no longer describes.

## Implementation Tasks

1. **SDK contract**
   1. 1.1 Rename `UdfContext::num_columns` to `input_column_count` with no forwarding alias, and update every implementation and call site across `crates/exasol-udf-sdk`, `crates/exa-udf-runtime`, `test-udfs/`, `benches/bench-udfs`, and `docs/writing-a-udf.md` [expert]
   2. 1.2 Add the defaulted `UdfContext::rows_in_group()` accessor and the matching `TestContext` setter
   3. 1.3 Add `UdfRun::generate_sql_for_import_spec` and `UdfRun::generate_sql_for_export_spec` with the `(ctx, json_spec)` signature and `Unimplemented` defaults
   4. 1.4 Change both spec-generation `ExaUdfVTable` slots to `(ctx, json_spec, result)` and bump `EXA_UDF_ABI_VERSION` to 10 [expert]
   5. 1.5 Extend the `#[exasol_udf]` annotation parser with `import_spec(...)` and `export_spec(...)`, generalize the vs-adapter shim builder to emit all three context-taking shims, and update the unknown-section error message [expert]
   6. 1.6 Add the `InputType` and `OutputType` enums, the defaulted `UdfContext::input_type()` and `output_type()` accessors, and the matching `TestContext` setters
   7. 1.7 Add the non-default `import` and `export` features to `crates/exasol-udf-sdk/Cargo.toml`, each enabling `serde` and `serde_json` as optional workspace dependencies, and correct the `[workspace.dependencies]` comment that scopes `serde` to the benchmark suite
   8. 1.8 Add `crates/exasol-udf-sdk/src/spec.rs` with `ImportSpec` behind `import`, `ExportSpec` behind `export`, their `from_json` parsers, and `spec_tests.rs` beside it. The parameter mirror and the `ConnectionObject` deserialization compile under either feature alone, and the column mirror belongs to `import`

2. **Runtime spec-call path, fixtures, and live coverage**
   1. 2.1 Add `crates/exa-udf-runtime/src/spec_json.rs` serializing `ImportSpecificationRep` and `ExportSpecificationRep` to the pinned JSON shape, with `spec_json_tests.rs` beside it [expert]
   2. 2.2 Route both spec function ids through the context-threading path, passing the serialized JSON, and close the session when the matching specification message is absent [expert]
   3. 2.3 Switch `LoadedUdf::call_generate_sql_for_import_spec` and `call_generate_sql_for_export_spec` to `call_ctx_arg_hook`, and delete the now-unused `call_arg_hook`
   4. 2.4 Report the SDK hook name in `MT_UNDEFINED_CALL`, keeping the protobuf variant name for the `SC_FN_NIL` sentinel
   5. 2.5 Tighten `HostEvent::SingleCall` decode coverage in `crates/exa-zmq-protocol/src/loop_tests.rs` so an absent payload field stays `None`
   6. 2.6 Rewrite `crates/exa-udf-runtime/tests/single_call.rs::import_spec_hook_error_surfaces_as_run_error` to send an `import_specification` message, add the export-side and hook-name cases, and update the existing `MT_UNDEFINED_CALL` assertions at `crates/exa-udf-runtime/tests/single_call.rs:237` and `:1056` from `"SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC"` and `"SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL"` to the SDK hook names `generate_sql_for_export_spec` and `virtual_schema_adapter_call`
   7. 2.7 Update `test-udfs/single-call-fixture` to the three-argument spec slots and echo the received `json_spec`
   8. 2.8 Add the `test-udfs/import-export-spec` fixture crate with its four entry points and `lib_tests.rs`, depending on `exasol-udf-sdk` with both the `import` and `export` features so it parses each `json_spec` through the typed structs, plus its `exa-udf-runtime` dev-dependency entry, its root `Cargo.toml` `members` and `default-members` entries, and its CI `-p` allowlist line
   9. 2.9 Add the live-DB `IMPORT FROM SCRIPT` scenario to `crates/it/tests/db_roundtrip.rs`, with the CONNECTION object and target column list it needs, asserting that the inserted rows carry the column count and each declared column name and type `IMPORT_WORKER` read through `ctx.input_column_count()` and `ctx.input_column(idx)`
   10. 2.10 Add the live-DB `EXPORT INTO SCRIPT` scenario to `crates/it/tests/db_roundtrip.rs`, with the source table whose column names it asserts
   11. 2.11 Document the spec-generation hooks, the `json_spec` shape, the `import` and `export` features, and the `input_type` and `output_type` accessors in `docs/writing-a-udf.md`

3. **Context metadata**
   1. 3.1 Carry `rows_in_group` from the input batch into `InputRowSet` and surface it on `HostContextBridge`
   2. 3.2 Surface `input_type` and `output_type` on `HostContextBridge` and `SingleCallContext` from the declared handshake axes, keeping one field per axis so the accessors and the existing `emit` and `next` gates read the same value
   3. 3.3 Add the `test-udfs/rows-in-group` fixture with its `lib_tests.rs`, dev-dependency entry, root `Cargo.toml` `members` and `default-members` entries, and CI `-p` allowlist line
   4. 3.4 Add the live-DB group-row-count scenario to `crates/it/tests/db_roundtrip.rs`

4. **Release hygiene**
   1. 4.1 Bump `[workspace.package].version` to the next minor, update the pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` to match, and commit the regenerated `Cargo.lock`. The bump changes `EXA_SDK_FINGERPRINT`, so every `test-udfs/*.so` MUST be rebuilt before the Integration checklist step runs

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: SDK contract | 1.1-1.8 | — | spec deltas `sdk/udf-sdk`, `sdk/udf-abi`, `sdk/udf-macro`; `crates/exasol-udf-sdk/src/`, `crates/exasol-udf-sdk/Cargo.toml`, the `[workspace.dependencies]` comment in the root `Cargo.toml`, `crates/exasol-udf-macros/src/lib.rs`, `crates/exasol-udf-macros/tests/`, plus the rename's call sites in `crates/exa-udf-runtime/src/rowset.rs`, `test-udfs/`, `benches/bench-udfs/`, `docs/writing-a-udf.md` |
| B: Runtime spec-call path | 2.1-2.11 | A (ABI slot shape, the `import` and `export` features task 2.8 enables, `docs/writing-a-udf.md`) | spec deltas `runtime/dispatch-single-call`, `protocol/single-call`, `examples/test-udfs`; `crates/exa-udf-runtime/src/{spec_json.rs,single_call.rs,loader.rs}`, `crates/exa-udf-runtime/tests/single_call.rs`, `crates/exa-zmq-protocol/src/loop_tests.rs`, `test-udfs/{single-call-fixture,import-export-spec}/`, `crates/it/tests/db_roundtrip.rs`, `.github/workflows/ci.yml`, `Cargo.toml` |
| C: Context metadata | 3.1-3.4 | A (trait methods, `crates/exa-udf-runtime/src/rowset.rs`), B (`crates/it/tests/db_roundtrip.rs`, `.github/workflows/ci.yml`, `examples/test-udfs` delta) | spec deltas `runtime/rowset-codec`, `examples/test-udfs`; `crates/exa-udf-runtime/src/rowset.rs`, `test-udfs/rows-in-group/`, `crates/it/tests/db_roundtrip.rs`, `.github/workflows/ci.yml`, `Cargo.toml` |
| D: Release hygiene | 4.1 | A, B, C | `Cargo.toml`, `Cargo.lock` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/exa-udf-runtime/src/loader.rs::call_arg_hook` | both remaining callers move to `call_ctx_arg_hook` |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| sdk/udf-sdk: UdfContext exposes typed accessors and row iteration | Unit | `crates/exasol-udf-sdk/src/context_tests.rs` | `typed_accessors_read_the_current_row` |
| sdk/udf-sdk: UdfRun default single-call hooks return Unimplemented | Unit | `crates/exasol-udf-sdk/src/context_tests.rs` | `udf_run_spec_hooks_default_to_unimplemented` |
| sdk/udf-sdk: Spec-generation hooks receive the specification as a JSON mirror of the proto message | Unit | `crates/exa-udf-runtime/src/spec_json_tests.rs` | `spec_json_mirrors_every_proto_field` |
| sdk/udf-sdk: UdfContext reports the row count of the current input group | Unit | `crates/exasol-udf-sdk/src/context_tests.rs` | `rows_in_group_defaults_to_zero` |
| sdk/udf-sdk: UdfContext reports the declared input and output iteration axes | Unit | `crates/exasol-udf-sdk/src/context_tests.rs` | `iteration_axis_accessors_default_to_none` |
| sdk/udf-sdk: The import and export features parse the specification payload into typed structs | Unit | `crates/exasol-udf-sdk/src/spec_tests.rs` | `import_and_export_spec_parse_the_pinned_json_shape` |
| sdk/udf-sdk: The test-support feature ships a reusable UdfContext test double | Unit | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `test_context_exposes_input_column_count_and_rows_in_group` |
| sdk/udf-sdk: The test-support feature ships a defaults-preserving UdfContext double | Unit | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `defaults_ctx_overrides_no_provided_method` |
| sdk/udf-abi: import_spec and export_spec annotations wire the spec-generation slots | Integration | `crates/exasol-udf-macros/tests/spec_hooks.rs` | `spec_annotations_wire_both_vtable_slots` |
| sdk/udf-abi: An omitted spec annotation leaves its slot None | Integration | `crates/exasol-udf-macros/tests/spec_hooks.rs` | `omitted_spec_annotations_leave_both_slots_none` |
| sdk/udf-abi: Spec-generation vtable slots take the context pointer, bumping the ABI version | Unit | `crates/exasol-udf-sdk/src/abi_tests.rs` | `spec_slots_take_context_and_abi_version_is_ten` |
| sdk/udf-macro: name attribute overrides the SQL entry point name | Integration | `crates/exasol-udf-macros/tests/spec_hooks.rs` | `name_combines_with_spec_sections` |
| sdk/udf-macro: Macro rejects an unknown annotation section by name | Integration | `crates/exasol-udf-macros/tests/trybuild/` | `unknown_annotation_section.rs` |
| protocol/single-call: Single-call request surfaces a SingleCall host event | Unit | `crates/exa-zmq-protocol/src/loop_tests.rs` | `single_call_event_carries_both_specification_messages` |
| runtime/dispatch-single-call: An import or export spec call delivers the serialized specification to the hook | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `import_spec_call_delivers_serialized_specification` |
| runtime/dispatch-single-call: Spec-generation hooks receive a SingleCallContext | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `export_spec_hook_reads_handshake_metadata_from_context` |
| runtime/dispatch-single-call: Unimplemented single-call hook replies MT_UNDEFINED_CALL | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `undefined_call_names_the_sdk_hook` |
| runtime/dispatch-single-call: IMPORT FROM SCRIPT runs the generated SQL against its worker UDF | Integration | `crates/it/tests/db_roundtrip.rs` | `import_from_script_roundtrip` |
| runtime/dispatch-single-call: EXPORT INTO SCRIPT surfaces the specification its hook observed | Integration | `crates/it/tests/db_roundtrip.rs` | `export_into_script_surfaces_spec` |
| runtime/rowset-codec: InputRowSet carries the group row count of the input batch | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `rows_in_group_is_carried_from_the_input_batch` |
| runtime/rowset-codec: The host context reports the iteration axes the database declared | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `context_reports_the_declared_iteration_axes` |
| runtime/rowset-codec: The database reports a non-zero group row count over a live connection | Integration | `crates/it/tests/db_roundtrip.rs` | `rows_in_group_reports_live_group_size` |
| examples/test-udfs: import-export-spec generates IMPORT and EXPORT SQL from the spec payload | Unit | `test-udfs/import-export-spec/src/lib_tests.rs` | `spec_hooks_build_worker_sql_from_json_spec` |
| examples/test-udfs: import-export-spec's worker reads a variadic input schema at runtime | Unit | `test-udfs/import-export-spec/src/lib_tests.rs` | `worker_builds_row_from_the_runtime_schema` |
| examples/test-udfs: rows-in-group reports the group row count the database sent | Unit | `test-udfs/rows-in-group/src/lib_tests.rs` | `emits_reported_and_iterated_counts` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-sdk | `cargo test -p exasol-udf-sdk --all-features` | 0 failures |
| sdk/udf-sdk default build | `cargo tree -p exasol-udf-sdk --edges normal` | no `serde` and no `serde_json` entry |
| sdk/udf-sdk one feature at a time | `cargo build -p exasol-udf-sdk --features import && cargo build -p exasol-udf-sdk --features export` | both exit 0 |
| sdk/udf-abi | `cargo test -p exasol-udf-sdk --all-features abi` | `EXA_UDF_ABI_VERSION` asserted as 10, 0 failures |
| sdk/udf-macro | `cargo test -p exasol-udf-macros` | trybuild case passes, 0 failures |
| protocol/single-call | `cargo test -p exa-zmq-protocol` | 0 failures |
| runtime/dispatch-single-call | `cargo test -p exa-udf-runtime --all-features single_call` | 0 failures |
| runtime/rowset-codec | `cargo test -p exa-udf-runtime --all-features rowset` | 0 failures |
| examples/test-udfs | `cargo build --release -p import-export-spec -p rows-in-group` | `target/release/libimport_export_spec.so` and `target/release/librows_in_group.so` exist |
| End-to-end IMPORT and EXPORT | `cargo test -p it --features integration` | `[it] scenario import_from_script ok`, `[it] scenario export_into_script ok`, and `[it] scenario rows_in_group ok` on stderr |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
