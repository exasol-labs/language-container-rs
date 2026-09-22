# Code Review Findings: add-import-export-spec-hooks

## Summary
- Files reviewed: 45
- Total findings: 18 (standard: 14, expert: 4)

Baseline verified before review: `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo test --all-features` 0 failures, `cargo build -p exasol-udf-sdk --features import` and `--features export` both exit 0, `cargo tree -p exasol-udf-sdk --edges normal` shows neither `serde` nor `serde_json`. Every finding below is a quality defect in green code, not a build break.

## Standard fixes

### crates/exasol-udf-sdk/src/context.rs

#### [MISSING_DOC_COMMENT] `pub trait UdfRun` lost its doc comment
- Location: line 341
- Issue: the diff deletes `/// Per-call lifecycle hooks — default implementations return Unimplemented for v1 single-call hooks` and puts nothing in its place, so `UdfRun` — a public trait of the published `exasol-udf-sdk` crate, and the one an author implements — now has no doc comment at all while every one of its methods has one. `grep -rn 'Per-call lifecycle hooks'` over the repo returns nothing, confirming the text was not relocated.
- Fix: In crates/exasol-udf-sdk/src/context.rs, add a doc comment directly above `pub trait UdfRun` (line 341) stating what the trait promises and its design intent: it is the per-call lifecycle surface the `#[exasol_udf]` macro wires into the vtable; `run` is required, and every single-call hook (`virtual_schema_adapter_call`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`) defaults to `Err(UdfError::Unimplemented(..))` so the runtime replies `MT_UNDEFINED_CALL` for a hook the author did not write.

### crates/exa-udf-runtime/src/single_call.rs

#### [MISSING_DESIGN_INTENT] `run_single_call` lost the doc comment that carried the single-call wire contract
- Location: line 9
- Issue: the diff removes the 12-line doc comment describing the single-call session (DB answers each `MT_RUN` with one `MT_CALL`; exactly one `MT_RETURN` or `MT_UNDEFINED_CALL` per call; the session ends on `MT_CLEANUP` followed by `MT_FINISHED`; the strict REQ/REP lockstep that makes one function call cost two exchanges) and replaces it with nothing. `grep -rn 'strict REQ/REP lockstep'` over the repo returns nothing, so the rationale is gone, not moved. Nothing in this change required removing it — the function's contract is unchanged.
- Fix: In crates/exa-udf-runtime/src/single_call.rs, restore the doc comment above `pub fn run_single_call` (line 9) from `git show HEAD:crates/exa-udf-runtime/src/single_call.rs`, keeping its wording verbatim.

#### [UNTESTED_ERROR_PATH] The export-side missing-specification close has no test
- Location: line 152 (`missing_specification("generate_sql_for_export_spec", "export_specification")`)
- Issue: `crates/exa-udf-runtime/tests/single_call.rs` covers the import side with `import_spec_call_without_its_specification_closes_the_session` (line 988), but no test drives `ScFnGenerateSqlForExportSpec` at a UDF whose export slot *is* registered while `export_specification` is `None`. The export arm of `missing_specification` therefore never executes in any test — the only export-side coverage is the success path (`export_spec_hook_reads_handshake_metadata_from_context`, line 1034) and the unregistered-slot path (`undefined_call_names_the_sdk_hook`, line 261). Plan task 2.6 called for the export-side case.
- Fix: In crates/exa-udf-runtime/tests/single_call.rs, add `export_spec_call_without_its_specification_closes_the_session`, modelled on `import_spec_call_without_its_specification_closes_the_session` (line 988): load the `import_export_spec` fixture (its export slot is registered), handshake, send `export_spec_call(conn_id, None)`, and assert the next request is `MT_Close` whose `exception_message` contains both `F-UDF-CL-RUST-9001` and `export_specification`, and does not contain `EXPORT_SPEC`.

#### [TOO_MANY_ARGUMENTS] `handshake_as` takes five arguments
- Location: crates/exa-udf-runtime/tests/single_call.rs line 100
- Issue: `handshake_as(server, conn_id, source, script_name, script_schema)` takes five parameters, over the three-argument guardrail; `handshake` (line 94) is a wrapper that supplies two defaults. Test code follows the same guardrails as production code.
- Fix: In crates/exa-udf-runtime/tests/single_call.rs, introduce a `struct ScriptIdentity { name: &'static str, schema: &'static str }` with a `Default` impl yielding `{ name: "SINGLE_CALL_UDF", schema: "" }`, change `handshake_as` to `fn handshake_as(server: &zmq::Socket, conn_id: u64, source: &str, identity: ScriptIdentity)`, and update the two call sites at lines 922 and 1047 plus the `handshake` wrapper at line 94.

### crates/exasol-udf-macros/src/lib.rs

#### [TOO_MANY_ARGUMENTS] `build_ctx_hook_tokens` takes four arguments
- Location: line 410
- Issue: generalizing `build_vs_adapter_tokens` grew it from three parameters to four (`path`, `write_c_string_ident`, `shim_ident`, `hook_name`), over the three-argument guardrail. `shim_ident` is redundant: each of the three call sites builds it with `format_ident!("__exa_<hook>_shim_{udf_name}")`, so the same identifier is derivable inside the function from `hook_name` and the UDF name.
- Fix: In crates/exasol-udf-macros/src/lib.rs, drop the `shim_ident` parameter from `build_ctx_hook_tokens` (line 410), add a `udf_name: &proc_macro2::Ident` parameter in its place, and derive the shim identifier inside the function from `hook_name` and `udf_name` with `format_ident!`; update the three call sites (lines 252-271) to stop constructing the identifier.

### docs/writing-a-udf.md

#### [OUTDATED_COMMENT] The `import_sql` example interpolates the raw `json_spec` into a SQL literal, contradicting the section's own secret-handling rule
- Location: line 877
- Issue: the canonical example authors will copy is `Ok(format!("SELECT {}.import_worker('{json_spec}')", ctx.script_schema()))`. Line 919 of the same section states that `json_spec` carries `connection_information`, which holds the CONNECTION object's password, "so never log the payload" — yet the example embeds that payload verbatim into a statement the database parses, executes and records. The example is also simply broken for any `WITH` value or password containing an apostrophe, because nothing escapes the literal; the repo's own fixture `test-udfs/import-export-spec/src/lib.rs` had to add `fn quote` (line 113) for exactly this reason.
- Fix: In docs/writing-a-udf.md, rewrite the `import_sql` snippet at lines 874-878 so it does not interpolate `json_spec` into the generated SQL: parse the payload and pass only the derived, non-secret values the worker needs (as the `spec.parameters` example at lines 936-948 already does), and add one sentence after the snippet stating that any value placed into a generated SQL literal must have its apostrophes doubled, since `WITH` values are whatever the statement's author typed.

#### [TOO_MANY_ARGUMENTS] `assert_import_row` takes four arguments
- Location: crates/it/tests/db_roundtrip.rs line 3141
- Issue: `assert_import_row(row, declared_columns, runtime_schema, statement)` takes four parameters, over the three-argument guardrail, and three of them are all `&str` in a row, so a transposed call at either call site would still compile.
- Fix: In crates/it/tests/db_roundtrip.rs, change `assert_import_row` (line 3141) to `fn assert_import_row(row: &str, statement: &str, expected_fragments: &[&str]) -> Result<()>` that iterates `expected_fragments` after the three always-present fragments, and update the two call sites (lines 3105 and 3128) to pass `&[declared_columns, RUNTIME_SCHEMA]`.

#### [REDUNDANT_COMMENT] Inline comment repeats the scenario's own doc comment
- Location: crates/it/tests/db_roundtrip.rs line 3120
- Issue: `// The subselect form is what populates the declared column list.` restates a sentence already in `import_from_script_roundtrip`'s doc comment ("the subselect form is the only one for which the database populates `subselect_column_specification`"), and the statement immediately below reads `IMPORT INTO (spec …, schema_info …)`. The user asked for no comments as the default style for this change.
- Fix: In crates/it/tests/db_roundtrip.rs, delete the inline comment at line 3120.

### crates/exasol-udf-sdk/src/spec_tests.rs

#### [REDUNDANT_COMMENT] Three inline comments restate the assertions beneath them
- Location: lines 61-62, 69, 108
- Issue: `// The enum keeps its proto variant name: the proto-to-ExaType mapping has exactly one owner, and it is not this crate.` duplicates the module doc of `spec.rs` (lines 9-12) and precedes `assert_eq!(columns[0].r#type.as_deref(), Some("PB_DOUBLE"))`, which states it. `// \`parameters\` is an ordered array, so a duplicate key survives the mirror.` precedes an assertion on `vec![("FILE", "a.csv"), ("FILE", "b.csv")]` that says the same. `// A pinned field carrying the wrong JSON type is a parse error too.` precedes `ExportSpec::from_json(r#"{"has_truncate": "yes"}"#).unwrap_err()`. The user asked for no comments as the default style for this change.
- Fix: In crates/exasol-udf-sdk/src/spec_tests.rs, delete the inline comments at lines 61-62, 69 and 108, leaving the assertions unchanged.

### crates/exa-udf-runtime/src/spec_json_tests.rs

#### [REDUNDANT_COMMENT] Inline comment inside the `json!` literal restates the literal
- Location: line 81
- Issue: `// An ordered array, not a map: the map form drops the second FILE.` sits inside the `serde_json::json!` expected value, directly above the two-element `"parameters"` array that shows exactly that. The same point is already made in the module doc of `spec_json.rs`.
- Fix: In crates/exa-udf-runtime/src/spec_json_tests.rs, delete the inline comment at line 81.

### crates/exa-udf-runtime/src/rowset_tests.rs

#### [INLINE_COMMENT] Inline comments narrate `rows_in_group_is_carried_from_the_input_batch`
- Location: lines 973-974, 979, 1005-1006
- Issue: three inline comments narrate the test step by step. `// A grouped batch reports its own declared count.` precedes a literal `rows_in_group: 7` and an assertion on `7`. `// No group defined: …` and `// First \`next()\` only marks the cursor started; …` restate what the following assertions and their messages already say; the test already carries a doc comment covering the same ground.
- Fix: In crates/exa-udf-runtime/src/rowset_tests.rs, delete the inline comments at lines 973-974, 979 and 1005-1006; if the batch-boundary point is worth keeping, fold it into the existing `assert!` message on the second `bridge.next()` call rather than a comment.

### crates/exasol-udf-sdk/src/test_support_tests.rs

#### [REDUNDANT_COMMENT] Inline comment restates the two assertions below it
- Location: line 73
- Issue: `// The column count follows the supplied rows; the group size is whatever the caller set, because no row data implies it.` precedes `assert_eq!(ctx.input_column_count(), 2)` and `assert_eq!(ctx.rows_in_group(), 9)` on a context built as `TestContext::set(vec![vec![Value::Int64(1), Value::Int64(2)]]).with_rows_in_group(9)`, which shows it. The same sentence already appears in the `TestContext` doc comment in `test_support.rs`.
- Fix: In crates/exasol-udf-sdk/src/test_support_tests.rs, delete the inline comment at lines 73-74.

### crates/exasol-udf-macros/tests/spec_hooks.rs

#### [REDUNDANT_COMMENT] Two inline comments restate their assertions
- Location: lines 76-77, 87
- Issue: `// DefaultsCtx reports the trait default for script_schema (empty), which is enough to prove the context pointer reached the annotated function.` precedes `assert_eq!(sql, r#"SELECT * FROM .WORKER(…)"#)`, whose empty schema position shows it. `// An \`Err\` from the annotated function surfaces as rc 1 plus its text.` precedes `assert_eq!(rc, 1)` and `assert!(text.contains("empty export specification"))`.
- Fix: In crates/exasol-udf-macros/tests/spec_hooks.rs, delete the inline comments at lines 76-77 and 87.

### test-udfs/import-export-spec/src/lib.rs

#### [INLINE_COMMENT] Inline comment inside the private `import_sql`
- Location: lines 64-65
- Issue: `// The derived table's aliases and casts are what give the variadic worker a predictable input schema to report back.` is an inline comment on a private function; the guardrails forbid both. The point belongs with the module doc, which already explains the fixture's two-worker design.
- Fix: In test-udfs/import-export-spec/src/lib.rs, delete the inline comment at lines 64-65 and, if the rationale is worth keeping, add one sentence to the module doc at lines 1-12 stating that the generated derived table aliases and casts both columns so `IMPORT_WORKER`'s variadic input has a fixed shape to report.

### test-udfs/rows-in-group/src/lib_tests.rs

#### [VAGUE_TEST_NAME] One test covers two behaviours; the second is unnamed and unmessaged
- Location: lines 4-30
- Issue: `emits_reported_and_iterated_counts` asserts the three-row group, then appends a second arrange/act/assert block for the empty group (lines 24-29) with an explanatory inline comment and an `assert_eq!` carrying no failure message. The name states neither condition, so a failure in the second block reports a test whose name describes the first. This is the only boundary case (empty group) in the fixture's suite, and it is hidden inside another test.
- Fix: In test-udfs/rows-in-group/src/lib_tests.rs, split lines 21-29 out into a second `#[test] fn reports_the_declared_count_for_an_empty_group()`, give its `assert_eq!` a failure message stating that `rows_in_group` is read before the first `next()` and is not derived from the number of rows iterated, delete the inline comment at lines 21-23, and rename the remaining test to state its condition (for example `reports_the_group_size_and_the_rows_it_iterated`).

## Expert fixes

### crates/exa-udf-runtime/src/rowset.rs

#### [INFORMATION_LEAKAGE] The handshake's iteration axes travel outside `HandshakeMeta`
- Location: lines 1838-1890 (`SingleCallContext` fields, `new`, `configure_iter_axes`); crates/exa-udf-runtime/src/single_call.rs lines 20-21, 112-119, 194-201, 224
- Issue: `HandshakeMeta` (line 1418) is the single snapshot of the handshake: it already carries `session_id`, `node_count`, `script_schema` and ten more fields, and `impl From<&UdfMeta> for HandshakeMeta` (line 1434) builds it from the same `UdfMeta` that owns `input_iter()` / `output_iter()`. This change reads those two axes separately in `run_single_call` (single_call.rs lines 20-21) and threads them as two loose parameters through `invoke_hook` and `invoke_ctx_hook` before installing them with a setter. One decision — what the handshake declared — now has two carriers, and every function between the handshake and the context must know about both. Adding a third handshake-derived value to a single-call hook would repeat the whole thread.
- Fix: In crates/exa-udf-runtime/src/rowset.rs, add `input_iter: IterType` and `output_iter: IterType` fields to `pub struct HandshakeMeta` (line 1418) and populate them in `impl From<&UdfMeta> for HandshakeMeta` (line 1434) from `meta.input_iter()` / `meta.output_iter()`; make `SingleCallContext::input_type()` / `output_type()` read `self.handshake.input_iter` / `self.handshake.output_iter` and delete the `input_iter` / `output_iter` fields and the `configure_iter_axes` method (lines 1845-1890). Then in crates/exa-udf-runtime/src/single_call.rs delete the `input_iter` / `output_iter` locals (lines 20-21), the two parameters on `invoke_hook` (line 112) and `invoke_ctx_hook` (line 194), the four call-site argument pairs, and the `bridge.configure_iter_axes(...)` call (line 224). Update `crates/exa-udf-runtime/src/rowset_tests.rs::context_reports_the_declared_iteration_axes` (line ~1030) and the `single_call_ctx()` helper to build the axes through `HandshakeMeta` instead of calling `configure_iter_axes`, and check every other `HandshakeMeta { .. }` literal in `rowset_tests.rs` still compiles.

#### [TACTICAL_SHORTCUT] `SingleCallContext::new` seeds the axes with a wrong answer that only a follow-up call corrects
- Location: lines 1845-1851, 1866-1874, 1883-1886
- Issue: `new` sets both axes to `IterType::Multiple`, so an instance nobody configures answers `Some(InputType::Set)` and `Some(OutputType::Emits)` — a specific, wrong declaration rather than the `None` the SDK doc promises for "a context carrying no host metadata" (`crates/exasol-udf-sdk/src/context.rs` lines 262-276). Correctness depends entirely on an undocumented ordering contract: every construction site must call `configure_iter_axes` immediately afterwards, which the guardrails forbid ("if a second call requires a first, make it unreachable without it"). `rowset_tests.rs::context_reports_the_declared_iteration_axes` only exercises the configured path, so a construction site that skips the setter passes the whole suite and ships a wrong axis to a spec-generation hook. The field's own doc comment (line 1848) admits the default exists for tests that never configure it.
- Fix: Covered by the `[INFORMATION_LEAKAGE]` fix above — moving both axes into `HandshakeMeta` makes them constructor-injected and removes the setter, so no unconfigured state exists. In addition, add a test to crates/exa-udf-runtime/src/rowset_tests.rs asserting that a `SingleCallContext` built from a `HandshakeMeta` derived from a SCALAR/RETURNS `UdfMeta` reports `Some(InputType::Scalar)` and `Some(OutputType::Returns)`, so the axes cannot silently default to `Set`/`Emits` again.

### crates/exa-udf-runtime/src/single_call.rs

#### [TOO_MANY_ARGUMENTS] `invoke_hook` and `invoke_ctx_hook` each take seven arguments
- Location: lines 112-120 and 194-203
- Issue: `invoke_hook(transport, proto, udf, call, handshake, input_iter, output_iter)` and `invoke_ctx_hook(transport, proto, handshake, input_iter, output_iter, arg, call)` both take seven parameters, more than double the guardrail. The `SingleCallRequest` struct introduced by this change groups the wire payload correctly but the surrounding call context — transport, protocol, handshake, and two axes — was left as loose positional arguments, so `invoke_ctx_hook`'s four consecutive non-`&str` arguments are reorderable without a compile error.
- Fix: In crates/exa-udf-runtime/src/single_call.rs, after applying the `[INFORMATION_LEAKAGE]` fix (which removes `input_iter` / `output_iter`), introduce `struct CallSession<'a> { transport: &'a ZmqTransport, proto: &'a mut Protocol, handshake: crate::rowset::HandshakeMeta }` built once in `run_single_call`, and reduce the two signatures to `fn invoke_hook(session: CallSession<'_>, udf: &LoadedUdf, call: SingleCallRequest) -> Result<HookOutcome, RuntimeError>` and `fn invoke_ctx_hook<F>(session: CallSession<'_>, arg: &str, call: F) -> Result<HookOutcome, RuntimeError>`; keep the `#[cfg(feature = "connect-back")]` handling of `transport`/`proto` inside `invoke_ctx_hook` unchanged. Re-run `cargo test -p exa-udf-runtime --all-features single_call` and `cargo clippy --all-targets --all-features -- -D warnings`.

### crates/exasol-udf-sdk/src/spec.rs

#### [INFORMATION_LEAKAGE] The `json_spec` shape is declared twice with only the export direction bound by a test
- Location: lines 24-88 (`Parameter`, `ColumnDefinition`, `ImportSpec`, `ExportSpec`); crates/exa-udf-runtime/src/spec_json.rs lines 26-86; crates/exasol-udf-sdk/src/connect_back.rs lines 23-32
- Issue: `spec_json.rs` claims to be "the single owner of that shape" (module doc line 6), but three modules now encode it independently: `spec_json.rs` writes the keys, `spec.rs` declares matching `serde` field names, and `connect_back.rs` derives `Deserialize` on `ConnectionObject` so its four field names must match `credentials()` in `spec_json.rs`. Nothing enforces the agreement: `spec_json_tests.rs` pins the emitted keys against one hand-written literal and `spec_tests.rs` parses a *different* hand-written literal, so renaming a key in `serialize_import` and updating only the runtime's own fixture leaves both suites green and breaks only against a live database. The export direction is in fact bound end to end — `crates/exa-udf-runtime/tests/single_call.rs::export_spec_hook_reads_handshake_metadata_from_context` (line 1034) drives the real serializer into the `import-export-spec` fixture, which parses with `ExportSpec::from_json`. The import direction has no such host-side test: `import_spec_call_delivers_serialized_specification` (line 914) uses `single_call_fixture`, which only echoes the JSON string without parsing it, so `serialize_import` → `ImportSpec::from_json` is exercised only by the live-DB scenario `import_from_script_roundtrip`, which does not run in `cargo test`.
- Fix: In crates/exa-udf-runtime/tests/single_call.rs, add `import_spec_call_parses_through_the_typed_spec` that loads the `import_export_spec` fixture (not `single_call_fixture`), handshakes with script schema `IT_RUST`, sends an `import_spec_call` carrying an `ImportSpecificationRep` with a populated `connection_name`, a two-element `subselect_column_specification`, and the duplicate-key `parameters()` pairs, and asserts the `MT_RETURN` `call_result.result` starts with `SELECT IT_RUST.IMPORT_WORKER(` and contains `conn=`, `params=[FILE=a.csv,FILE=b.csv]`, `is_subselect=`, and the declared column names — mirroring `export_spec_hook_reads_handshake_metadata_from_context` (line 1034) so both directions of the serializer/parser contract fail together when a key is renamed on one side.
