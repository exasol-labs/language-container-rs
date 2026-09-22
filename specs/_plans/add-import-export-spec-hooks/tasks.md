# Tasks: add-import-export-spec-hooks

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: SDK contract)
- [x] 1.1 Rename `UdfContext::num_columns` to `input_column_count` with no forwarding alias, and update every implementation and call site across `crates/exasol-udf-sdk`, `crates/exa-udf-runtime`, `test-udfs/`, `benches/bench-udfs`, and `docs/writing-a-udf.md` [expert]
- [x] 1.2 Add the defaulted `UdfContext::rows_in_group()` accessor and the matching `TestContext` setter
- [x] 1.3 Add `UdfRun::generate_sql_for_import_spec` and `UdfRun::generate_sql_for_export_spec` with the `(ctx, json_spec)` signature and `Unimplemented` defaults
- [x] 1.4 Change both spec-generation `ExaUdfVTable` slots to `(ctx, json_spec, result)` and bump `EXA_UDF_ABI_VERSION` to 10 [expert]
- [x] 1.5 Extend the `#[exasol_udf]` annotation parser with `import_spec(...)` and `export_spec(...)`, generalize the vs-adapter shim builder to emit all three context-taking shims, and update the unknown-section error message [expert]
- [x] 1.6 Add the `InputType` and `OutputType` enums, the defaulted `UdfContext::input_type()` and `output_type()` accessors, and the matching `TestContext` setters
- [x] 1.7 Add the non-default `import` and `export` features to `crates/exasol-udf-sdk/Cargo.toml`, each enabling `serde` and `serde_json` as optional workspace dependencies, and correct the `[workspace.dependencies]` comment that scopes `serde` to the benchmark suite
- [x] 1.8 Add `crates/exasol-udf-sdk/src/spec.rs` with `ImportSpec` behind `import`, `ExportSpec` behind `export`, their `from_json` parsers, and `spec_tests.rs` beside it. The parameter mirror and the `ConnectionObject` deserialization compile under either feature alone, and the column mirror belongs to `import`

## Phase 2: Implementation (Group B: Runtime spec-call path)
- [x] 2.1 Add `crates/exa-udf-runtime/src/spec_json.rs` serializing `ImportSpecificationRep` and `ExportSpecificationRep` to the pinned JSON shape, with `spec_json_tests.rs` beside it [expert]
- [x] 2.2 Route both spec function ids through the context-threading path, passing the serialized JSON, and close the session when the matching specification message is absent [expert]
- [x] 2.3 Switch `LoadedUdf::call_generate_sql_for_import_spec` and `call_generate_sql_for_export_spec` to `call_ctx_arg_hook`, and delete the now-unused `call_arg_hook`
- [x] 2.4 Report the SDK hook name in `MT_UNDEFINED_CALL`, keeping the protobuf variant name for the `SC_FN_NIL` sentinel
- [x] 2.5 Tighten `HostEvent::SingleCall` decode coverage in `crates/exa-zmq-protocol/src/loop_tests.rs` so an absent payload field stays `None`
- [x] 2.6 Rewrite `crates/exa-udf-runtime/tests/single_call.rs::import_spec_hook_error_surfaces_as_run_error` to send an `import_specification` message, add the export-side and hook-name cases, and update the existing `MT_UNDEFINED_CALL` assertions at `crates/exa-udf-runtime/tests/single_call.rs:237` and `:1056` from `"SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC"` and `"SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL"` to the SDK hook names `generate_sql_for_export_spec` and `virtual_schema_adapter_call`
- [x] 2.7 Update `test-udfs/single-call-fixture` to the three-argument spec slots and echo the received `json_spec`
- [x] 2.8 Add the `test-udfs/import-export-spec` fixture crate with its four entry points and `lib_tests.rs`, depending on `exasol-udf-sdk` with both the `import` and `export` features so it parses each `json_spec` through the typed structs, plus its `exa-udf-runtime` dev-dependency entry, its root `Cargo.toml` `members` and `default-members` entries, and its CI `-p` allowlist line
- [x] 2.9 Add the live-DB `IMPORT FROM SCRIPT` scenario to `crates/it/tests/db_roundtrip.rs`, with the CONNECTION object and target column list it needs, asserting that the inserted rows carry the column count and each declared column name and type `IMPORT_WORKER` read through `ctx.input_column_count()` and `ctx.input_column(idx)`
- [x] 2.10 Add the live-DB `EXPORT INTO SCRIPT` scenario to `crates/it/tests/db_roundtrip.rs`, with the source table whose column names it asserts
- [x] 2.11 Document the spec-generation hooks, the `json_spec` shape, the `import` and `export` features, and the `input_type` and `output_type` accessors in `docs/writing-a-udf.md`

## Phase 2: Implementation (Group C: Context metadata)
- [x] 3.1 Carry `rows_in_group` from the input batch into `InputRowSet` and surface it on `HostContextBridge`
- [x] 3.2 Surface `input_type` and `output_type` on `HostContextBridge` and `SingleCallContext` from the declared handshake axes, keeping one field per axis so the accessors and the existing `emit` and `next` gates read the same value
- [x] 3.3 Add the `test-udfs/rows-in-group` fixture with its `lib_tests.rs`, dev-dependency entry, root `Cargo.toml` `members` and `default-members` entries, and CI `-p` allowlist line
- [x] 3.4 Add the live-DB group-row-count scenario to `crates/it/tests/db_roundtrip.rs`

## Phase 2: Implementation (Group D: Release hygiene)
- [x] 4.1 Bump `[workspace.package].version` to the next minor, update the pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` to match, and commit the regenerated `Cargo.lock`. The bump changes `EXA_SDK_FINGERPRINT`, so every `test-udfs/*.so` MUST be rebuilt before the Integration checklist step runs

## Phase 4: Review Fixes
- [x] 4.2 Add a doc comment above `pub trait UdfRun` in `crates/exasol-udf-sdk/src/context.rs` stating it is the per-call lifecycle surface the `#[exasol_udf]` macro wires into the vtable, that `run` is required, and that every single-call hook defaults to `Err(UdfError::Unimplemented(..))` so the runtime replies `MT_UNDEFINED_CALL`
- [x] 4.3 Restore the single-call wire-contract doc comment above `pub fn run_single_call` in `crates/exa-udf-runtime/src/single_call.rs` verbatim from `git show HEAD:crates/exa-udf-runtime/src/single_call.rs`
- [x] 4.4 Add `export_spec_call_without_its_specification_closes_the_session` to `crates/exa-udf-runtime/tests/single_call.rs`, driving the `import_export_spec` fixture with `export_spec_call(conn_id, None)` and asserting the `MT_Close` message names `F-UDF-CL-RUST-9001` and `export_specification` but not `EXPORT_SPEC`
- [x] 4.5 Introduce `struct ScriptIdentity { name, schema }` with a `Default` impl in `crates/exa-udf-runtime/tests/single_call.rs`, reduce `handshake_as` to four parameters, and update the `handshake` wrapper and both explicit call sites
- [x] 4.6 Replace the `shim_ident` parameter of `build_ctx_hook_tokens` in `crates/exasol-udf-macros/src/lib.rs` with `udf_name`, derive the shim identifier inside the function from `hook_name` and `udf_name`, and update the three call sites
- [x] 4.7 Rewrite the `import_sql` snippet in `docs/writing-a-udf.md` so it parses `json_spec` and interpolates only derived non-secret values, and add one sentence on doubling apostrophes in generated SQL literals
- [x] 4.8 Change `assert_import_row` in `crates/it/tests/db_roundtrip.rs` to `fn assert_import_row(row: &str, statement: &str, expected_fragments: &[&str]) -> Result<()>` and update both call sites
- [x] 4.9 Delete the redundant inline comment above the subselect `IMPORT INTO` statement in `crates/it/tests/db_roundtrip.rs`
- [x] 4.10 Delete the three redundant inline comments in `crates/exasol-udf-sdk/src/spec_tests.rs`
- [x] 4.11 Delete the redundant inline comment inside the `json!` literal in `crates/exa-udf-runtime/src/spec_json_tests.rs`
- [x] 4.12 Delete the narrating inline comments in `rows_in_group_is_carried_from_the_input_batch` in `crates/exa-udf-runtime/src/rowset_tests.rs`, folding the batch-boundary point into the existing `assert!` message if it is worth keeping
- [x] 4.13 Delete the redundant inline comment in `crates/exasol-udf-sdk/src/test_support_tests.rs`
- [x] 4.14 Delete the two redundant inline comments in `crates/exasol-udf-macros/tests/spec_hooks.rs`
- [x] 4.15 Delete the inline comment in the private `import_sql` in `test-udfs/import-export-spec/src/lib.rs`, folding the rationale into the module doc if it is worth keeping
- [x] 4.16 Split the empty-group block of `emits_reported_and_iterated_counts` in `test-udfs/rows-in-group/src/lib_tests.rs` into `reports_the_declared_count_for_an_empty_group` with a failure message, delete its inline comment, and rename the remaining test to state its condition
- [x] 4.17 Move `input_iter` and `output_iter` into `HandshakeMeta` in `crates/exa-udf-runtime/src/rowset.rs`, delete `SingleCallContext`'s own axis fields and `configure_iter_axes`, and remove the axis locals, parameters, call-site arguments and setter call from `crates/exa-udf-runtime/src/single_call.rs` [expert]
- [x] 4.18 Add a test to `crates/exa-udf-runtime/src/rowset_tests.rs` asserting a `SingleCallContext` built from a SCALAR/RETURNS-derived `HandshakeMeta` reports `Some(InputType::Scalar)` and `Some(OutputType::Returns)` [expert]
- [x] 4.19 Introduce `struct CallSession<'a>` in `crates/exa-udf-runtime/src/single_call.rs` grouping transport, protocol and handshake, and reduce `invoke_hook` and `invoke_ctx_hook` to three and three parameters respectively [expert]
- [x] 4.20 Add `import_spec_call_parses_through_the_typed_spec` to `crates/exa-udf-runtime/tests/single_call.rs`, driving the real `serialize_import` into the `import_export_spec` fixture so the import direction of the serializer/parser contract is bound host-side [expert]

## Phase 3: Verification
- [x] 3.1 Run automated checks per plan `## Verification > Checklist`
- [x] 3.2 Scenario coverage audit
- [x] 3.3 Manual verification per plan `## Verification > Manual Testing`

## Phase 5 Notes

Corrected a wrong test assumption in `import_from_script_roundtrip`
(`crates/it/tests/db_roundtrip.rs`), found by running the scenario against a
live Exasol Docker DB. `IMPORT_WORKER` is a variadic `(...)` UDF with no
declared parameter names; the derived table `IMPORT_SPEC_GEN`'s `SELECT`
aliases the two columns as `SPEC`/`PARAM_COUNT`, but the engine reports the
variadic worker's input column *names* as positional indices (`0`, `1`), not
the derived table's aliases — only the *types* (`VARCHAR(2000) UTF8`,
`DECIMAL(9,0)`) survive as expected. Live error observed:

```
Error: IMPORT INTO the target table produced "conn=IT_IMPORT_CONN params=[PARAM_A=alpha,PARAM_B=beta] is_subselect=false cols=[] || cols=2 [0:VARCHAR(2000) UTF8,1:DECIMAL(9,0)]", which does not carry "cols=2 [SPEC:VARCHAR(2000) UTF8,PARAM_COUNT:DECIMAL(9,0)]"
```

Fixed by changing `RUNTIME_SCHEMA` in `db_roundtrip.rs` to
`"cols=2 [0:VARCHAR(2000) UTF8,1:DECIMAL(9,0)]"` and correcting the adjacent
comment and the `test-udfs/import-export-spec/src/lib.rs` module doc, which
both overclaimed that the derived table's aliases reach the variadic
worker's reported names. `ColumnInfo` (`crates/exasol-udf-sdk/src/value.rs`)
mirrors the DB's handshake wire metadata verbatim; there is no
name-computation logic in this codebase to fix, so no runtime or SQL-generation
code changed.

Corrected `rows_in_group_reports_live_group_size` (task 3.4,
`crates/it/tests/db_roundtrip.rs`), found by running the scenario against a
live Exasol Docker DB. The query paired an `EMITS`-declared SET UDF call with
a pass-through `g` column in the same `GROUP BY` select list; Exasol rejects
that combination outright. Live error observed:

```
Error: query: SELECT GROUP_CONCAT(TO_CHAR(g) || '=' || TO_CHAR(reported) || ':' || TO_CHAR(iterated) ORDER BY g) FROM (SELECT g, rows_in_group(x) FROM it_rust.rows_in_group_src GROUP BY g)

Caused by:
    Query execution failed: Protocol error: Select list containing an emitting setfunction may not have additional elements (Session: 1877050961521147904) (SQL state: 42000)
```

An `EMITS` SET UDF under `GROUP BY` must be the sole select-list element; the
engine treats a `RETURNS`-style SET UDF (a single-value aggregate, e.g.
`set_sum_multi_group_by` at `crates/it/tests/db_roundtrip.rs:2204`) differently
and allows it alongside a pass-through group-key column. Fixed by having
`rows_in_group` itself read the group key from input column 0
(`ctx.get_i64(0)?.unwrap_or_default()`, captured on the first `next()`) and
re-emit it as the leading output value, so the select list contains only the
UDF call: `SELECT rows_in_group(g, x) FROM ... GROUP BY g`. The script's
declared shape changed to `rows_in_group(g BIGINT, x BIGINT) EMITS (g BIGINT,
reported BIGINT, iterated BIGINT)`.
