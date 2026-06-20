# Plan: add-multi-udf-entry-points

## Summary

Allow a single compiled UDF `.so` to host multiple UDF entry points by giving every `#[exasol_udf]`-annotated function its own namespaced export `__exa_udf_entry_<NAME>`, where `<NAME>` is the SQL script name (derived from the function identifier in `UPPER_SNAKE_CASE`, or set verbatim via `name = "..."`). The runtime resolves the entry symbol by the script name the database sends in the handshake, hard-breaking old single-`__exa_udf_entry` artifacts with a clear "rebuild" error.

## Design

### Context

Today the `#[exasol_udf]` macro emits a fixed set of module-level symbols (`__EXA_INPUT_SCHEMA`, `__exa_run_shim`, `__EXA_VTABLE`, `__exa_udf_entry`, …). Because the names are fixed, a second annotation in the same crate is a guaranteed duplicate-symbol link error — so one `.so` can carry exactly one UDF. Authors who want several UDFs must ship several `.so` files and upload each separately, multiplying BucketFS objects and `CREATE SCRIPT` registrations.

The database already sends the bare script object name to the SLC during the handshake (`exascript_info.script_name`, proto field 3 → `UdfMeta.script_name`), but the loader ignores it and blindly resolves the single `__exa_udf_entry`. Threading that name through to a namespaced symbol lookup unlocks many-UDFs-per-`.so` with no protocol change.

- **Goals**
  - One `.so` may export many UDFs, each addressable by SQL script name.
  - Entry-point name derives from the Rust function name by default; `name = "..."` overrides it.
  - Loader resolves `__exa_udf_entry_<SCRIPT_NAME>` using the existing `UdfMeta.script_name`.
  - A missing entry point produces a clear, actionable error (rebuild hint), surfaced via the protocol close path.
- **Non-Goals**
  - No backward compatibility for the bare `__exa_udf_entry` symbol — this is a deliberate hard break (interview decision).
  - No registry symbol or runtime enumeration of UDFs in the protocol path (the loader looks up exactly one named symbol per session).
  - No change to the `localzmq+protobuf` wire protocol, the vtable layout, or the ABI version.
  - No schema-qualified name handling — `script_name` is the bare object name; schema (`script_schema`) is not part of the symbol.

### Decision

Namespace every macro-generated item with the derived SQL name and look it up by `script_name` at load time.

#### Architecture

```
CREATE SCRIPT DOUBLE_IT ... %udf_object lib.so
        │ (DB handshake: exascript_info.script_name = "DOUBLE_IT")
        ▼
 UdfMeta.script_name = "DOUBLE_IT"  ──────────────┐
        │                                          │
        ▼                                          ▼
 Runtime::run ──▶ LoadedUdf::open(path, "DOUBLE_IT")
                       │  dlsym("__exa_udf_entry_DOUBLE_IT")
                       │     ├─ found  ─▶ validate abi+fingerprint ─▶ vtable
                       │     └─ absent ─▶ "no entry point found for script 'DOUBLE_IT'; hint: rebuild with sdk >= 0.14.0"
                       ▼
              lib.so exports:
                __exa_udf_entry_DOUBLE_IT  ──▶ &__EXA_VTABLE_DOUBLE_IT
                __exa_udf_entry_TRIPLE_IT  ──▶ &__EXA_VTABLE_TRIPLE_IT
```

The macro derives `NAME` once, then suffixes every generated identifier (`__EXA_INPUT_SCHEMA_<NAME>`, `__EXA_OUTPUT_SCHEMA_<NAME>`, `__exa_write_c_string_<NAME>`, `__exa_run_shim_<NAME>`, `__exa_destroy_shim_<NAME>`, `__exa_vs_adapter_shim_<NAME>`, `__EXA_VTABLE_<NAME>`, `__exa_udf_entry_<NAME>`). Only `__exa_udf_entry_<NAME>` is `#[unsafe(no_mangle)]`; the rest are namespaced purely to avoid intra-crate collisions. `__exa_write_c_string_<NAME>` is duplicated per UDF (accepted code duplication for a tiny unsafe helper, avoids any cross-UDF symbol sharing).

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Name derivation (ident → `UPPER_SNAKE_CASE`, or verbatim `name`) | `exasol-udf-macros` | Single source of truth for the SQL name; matches DB `script_name` |
| Per-UDF symbol namespacing via `format_ident!` suffix | `exasol-udf-macros` | Eliminates duplicate-symbol errors so many UDFs coexist in one crate |
| Name-parameterized symbol lookup | `exa-udf-runtime::loader` | Resolves the exact UDF the DB asked for |
| Symbol enumeration (no script name available) | `cargo-exasol-udf` validate/build | CLI validates a `.so` without a DB; must discover all `__exa_udf_entry_*` |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Per-UDF `__exa_udf_entry_<NAME>` symbols | Single registry symbol returning a name→vtable table | Per-UDF symbols are a direct `dlsym` by script name — no table parse, no allocation, no registry ABI to version; matches the existing one-symbol loader shape |
| Hard-break legacy `__exa_udf_entry` (no fallback) | Keep emitting bare symbol + fall back when named symbol absent | Interview decision; a silent fallback would mask author error and load the wrong UDF when a `.so` has several; the rebuild-hint error is unambiguous |
| Name from fn ident, `UPPER_SNAKE_CASE` | Require explicit `name` always; keep struct ident verbatim | Zero-config ergonomics: `fn double_it` → `DOUBLE_IT` matches the natural `CREATE SCRIPT DOUBLE_IT`; `name=` covers the rest |
| Reuse existing `UdfMeta.script_name` | Add a new proto field / parse name from `%udf_object` | The field already exists and is populated from `exascript_info.script_name`; no protocol or parsing change needed |
| CLI enumerates `__exa_udf_entry_*` | CLI takes a `--name` argument | `validate`/`build` have no DB and no script name; enumerating proves every embedded UDF is ABI-compatible in one pass |
| MINOR bump 0.13.1 → 0.14.0 | PATCH bump | Pre-1.0 convention: MINOR signals a breaking change; authors must rebuild |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-macro | CHANGED | `specs/_plans/add-multi-udf-entry-points/sdk/udf-macro/spec.md` |
| runtime/dispatch-loader | CHANGED | `specs/_plans/add-multi-udf-entry-points/runtime/dispatch-loader/spec.md` |
| tools/cargo-exaudf | CHANGED | `specs/_plans/add-multi-udf-entry-points/tools/cargo-exaudf/spec.md` |
| examples/test-udfs | CHANGED | `specs/_plans/add-multi-udf-entry-points/examples/test-udfs/spec.md` |
| workspace/version | CHANGED | `specs/_plans/add-multi-udf-entry-points/workspace/version/spec.md` |

## Dependencies

- No new external crates. The macro already depends on `syn`/`quote`/`proc-macro2`; name derivation and `format_ident!` are within those.
- The loader already receives `UdfMeta` (which carries `script_name`); no protocol change.

## Migration

| Current | New |
|---------|-----|
| `.so` exports one `__exa_udf_entry` | `.so` exports one `__exa_udf_entry_<NAME>` per UDF |
| Loader resolves `__exa_udf_entry` | Loader resolves `__exa_udf_entry_<SCRIPT_NAME>` |
| Old `.so` loads against any script | Old `.so` fails: `no entry point found for script '<NAME>'; hint: rebuild with sdk >= 0.14.0` |
| Workspace `0.13.1` | Workspace `0.14.0` (SDK pin in sync) |

## Implementation Tasks

1. **Macro: parse `name = "..."` attribute**
   1.1 Add a `name: Option<String>` field to `Annotations` and parse the `name = "literal"` key in `Annotations::parse` (alongside `input`/`emits`/`vs_adapter`).
   1.2 Add an unknown-key error path update so the "expected" list mentions `name`.

2. **Macro: derive the SQL name and namespace every generated symbol** `[expert]`
   2.1 Compute `udf_name`: if `name` present use it verbatim, else `fn_ident.to_string().to_uppercase()`.
   2.2 Build suffixed identifiers with `format_ident!` for `__EXA_INPUT_SCHEMA_<NAME>`, `__EXA_OUTPUT_SCHEMA_<NAME>`, `__exa_write_c_string_<NAME>`, `__exa_run_shim_<NAME>`, `__exa_destroy_shim_<NAME>`, `__exa_vs_adapter_shim_<NAME>`, `__EXA_VTABLE_<NAME>`, `__exa_udf_entry_<NAME>`.
   2.3 Rewrite `build_schema_tokens`, `build_vs_adapter_tokens`, and the main `quote!` to use the suffixed identifiers; thread the name suffix into the shared `__exa_write_c_string_<NAME>` references inside both shims. Remove the bare `__exa_udf_entry`.

3. **Macro: update unit/trybuild tests**
   3.1 Update `crates/exasol-udf-macros/tests/annotation.rs`, `annotation_typed.rs`, `run_error.rs`, `vs_adapter.rs` to call the new named entry symbol (e.g. `__exa_udf_entry_<NAME>`).
   3.2 Repoint the `dup_entry` trybuild fixture to two same-name annotations; add a passing test (or fixture) proving two distinct-name annotations compile and export two symbols.

4. **Runtime loader: name-parameterized lookup**
   4.1 Change `LoadedUdf::open(path)` → `LoadedUdf::open(path, script_name: &str)`; build the symbol `__exa_udf_entry_<script_name>` and `dlsym` it.
   4.2 On absent symbol, return a `RuntimeError` whose message is `no entry point found for script '<NAME>'; hint: rebuild with sdk >= 0.14.0` (a dedicated variant or `Loader(..)` string).
   4.3 Update the caller in `crates/exa-udf-runtime/src/lib.rs` (`Runtime::run`) to pass `meta.script_name`.

5. **Runtime loader: update tests**
   5.1 Update `crates/exa-udf-runtime/tests/loader.rs` fixtures to export a named symbol and call `open(path, name)`; add a test for the missing-named-entry error and a legacy bare-`__exa_udf_entry` rejection test.
   5.2 Update any `connect_back.rs`/`main.rs` callers of `.open(...)` to pass a script name.

6. **cargo-exaudf: enumerate named entry points** `[expert]`
   6.1 In `crates/cargo-exasol-udf/src/validate.rs`, replace the single `__exa_udf_entry` lookup with enumeration of exported `__exa_udf_entry_*` symbols (read the `.so` dynamic symbol table — e.g. via `object`/`goblin` if available, or parse `nm`/`readelf` output already used elsewhere), validating each vtable's abi+fingerprint.
   6.2 In `crates/cargo-exasol-udf/src/build.rs`, replace the post-build `__exa_udf_entry` resolution with an "at least one `__exa_udf_entry_*`" check.
   6.3 Update `crates/cargo-exasol-udf/tests/validate.rs` expectations for the new messages.

7. **test-udfs fixtures**
   7.1 Add a second annotated function `fn annotated_double` to `test-udfs/annotated-fixture/src/lib.rs` (alongside `fn annotated`) so one crate exports two entry points.
   7.2 Fix `test-udfs/annotated-double/src/lib.rs` self-test that calls `__exa_udf_entry()` to call `__exa_udf_entry_ANNOTATED_DOUBLE()`.

8. **Version bump**
   8.1 Bump `[workspace.package].version` and the `exasol-udf-sdk` pin in `[workspace.dependencies]` from `0.13.1` to `0.14.0`; regenerate `Cargo.lock`.

9. **Docs**
   9.1 Update `docs/` (writing-a-udf / cargo-ecosystem) to document multiple UDFs per `.so`, the `name = "..."` attribute, the `UPPER_SNAKE_CASE` derivation rule, and the rebuild-required note.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A (macro) | 1, 2, 3 |
| Group B (runtime) | 4, 5 |
| Group C (CLI) | 6 |
| Group D (fixtures + version + docs) | 7, 8, 9 |

Sequential dependencies:
- Group A → Group B (loader tests build named-symbol fixtures; the macro contract must be settled first) — though loader fixtures hand-craft symbols, so B can largely proceed against the agreed symbol name `__exa_udf_entry_<NAME>`.
- Group A → Group D task 7 (fixtures depend on the macro emitting named symbols).
- Group A/B/C → Group D task 8 (version bump last, after code compiles).

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Token generation | `crates/exasol-udf-macros/src/lib.rs` (bare `__exa_udf_entry` fn) | Replaced by `__exa_udf_entry_<NAME>` |
| Symbol lookup | `crates/exa-udf-runtime/src/loader.rs` (`b"__exa_udf_entry\0"`) | Replaced by name-parameterized lookup |
| Symbol lookup | `crates/cargo-exasol-udf/src/validate.rs`, `build.rs` (`b"__exa_udf_entry\0"`) | Replaced by `__exa_udf_entry_*` enumeration |
| trybuild fixture | `crates/exasol-udf-macros/tests/trybuild/dup_entry.rs` | Reframed: same-name duplicate, not any-two-annotations |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| udf-macro / exasol_udf macro generates the entry point and vtable | Integration | `crates/exasol-udf-macros/tests/annotation.rs` | `entry_point_symbol_is_named` |
| udf-macro / run shim catches panics and returns an error code | Integration | `crates/exasol-udf-macros/tests/run_error.rs` | `run_shim_catches_panic` |
| udf-macro / function name is translated to UPPER_SNAKE_CASE SQL name | Integration | `crates/exasol-udf-macros/tests/annotation.rs` | `fn_name_uppercased_to_sql_name` |
| udf-macro / name attribute overrides the SQL entry point name | Integration | `crates/exasol-udf-macros/tests/annotation.rs` | `name_attribute_overrides_entry_name` |
| udf-macro / Two exasol_udf annotations with the same name fail to link | Integration (trybuild) | `crates/exasol-udf-macros/tests/trybuild/dup_entry.rs` | `trybuild compile-fail` |
| udf-macro / Two exasol_udf annotations with distinct names produce independent entry points | Integration | `crates/exasol-udf-macros/tests/annotation.rs` | `two_distinct_names_export_two_entries` |
| udf-macro / exasol_udf annotation with an unknown type fails to compile | Integration (trybuild) | `crates/exasol-udf-macros/tests/trybuild/` | `unknown_type compile-fail` |
| dispatch-loader / Loader accepts a matching .so and calls create | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_accepts_named_entry` |
| dispatch-loader / Loader returns clear error when named entry point is absent | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_errors_on_missing_named_entry` |
| dispatch-loader / Legacy single-entry .so fails to load | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_rejects_legacy_bare_entry` |
| dispatch-loader / Loader rejects an ABI version mismatch | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_rejects_abi_mismatch` |
| dispatch-loader / Loader rejects a fingerprint mismatch | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_rejects_fingerprint_mismatch` |
| dispatch-loader / Artifact path is parsed from the udf_object option | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `artifact_path_parsed` |
| dispatch-loader / JIT compilation is unsupported in v1 | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `jit_unsupported` |
| cargo-exaudf / validate accepts a compatible .so | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_accepts_named_entries` |
| cargo-exaudf / validate rejects an ABI or fingerprint mismatch | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_mismatch` |
| cargo-exaudf / validate rejects a .so missing any entry symbol | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_missing_entry` |
| cargo-exaudf / build verifies the artifact exports at least one named entry point | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `build_verifies_named_entry` |
| test-udfs / annotated-double declares its schema via the typed annotation | Integration | `crates/it/` (db-roundtrip) | `it_annotated_double` |
| test-udfs / annotated-fixture exports two named entry points from one .so | Integration | `crates/exa-udf-runtime/tests/` or `crates/it/` | `it_fixture_two_entries` |
| workspace/version / Workspace version is bumped to 0.14.0 ... | Integration | `crates/it/` or manual | `version_is_0_14_0` (grep assert) |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-macro | `cargo expand -p annotated-fixture` (or `nm -D target/.../libannotated_fixture.so | grep __exa_udf_entry`) | Two symbols: `__exa_udf_entry_ANNOTATED`, `__exa_udf_entry_ANNOTATED_DOUBLE`; no bare `__exa_udf_entry` |
| runtime/dispatch-loader | `cargo test -p exa-udf-runtime` | Loader tests pass; missing-entry test shows the rebuild-hint message |
| tools/cargo-exaudf | `cargo exasol-udf build` in a multi-UDF crate, then `cargo exasol-udf validate target/x86_64-unknown-linux-musl/release/lib<crate>.so` | Reports each discovered UDF name and exits 0 |
| examples/test-udfs | `cargo exasol-udf build` in `test-udfs/annotated-fixture` | Builds; `.so` exports both named entry points |
| workspace/version | `grep '^version' Cargo.toml` | `version = "0.14.0"` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Unit tests | `cargo test` | 0 failures |
| Integration tests | `cargo test -p it --features integration` | 0 failures (live Exasol Docker) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
