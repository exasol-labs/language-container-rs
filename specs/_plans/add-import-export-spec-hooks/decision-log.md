# Decision Log: add-import-export-spec-hooks

## Interview

**Q:** Issue #45 lists 3 blocking gaps (macro wiring, spec payload reaching the hook, `UdfContext` access) plus 3 non-blocking parity gaps (`rows_in_group`, `MT_UNDEFINED_CALL` naming, `num_columns` rename). Should this plan cover the whole issue, or just the blocking core?
**A:** The whole issue. Plan all six gaps, not just the three blocking ones.

**Q:** The `json_spec` payload the hooks receive is internal. No external system parses it. What shape should it take?
**A:** A full 1:1 mirror of the proto message (`ImportSpecificationRep` / `ExportSpecificationRep`). Serialize every field verbatim, not a hand-picked subset.

**Q:** Proposed ADR scope: only "spec-generation hooks get a `UdfContext` via the same double-indirection ABI pattern as `vs_adapter`" becomes an ADR. Everything else stays a plain decision-log entry.
**A:** Agreed as proposed.

## Design Decisions

### [1] Every context-taking single-call hook uses one slot shape

- **Decision:** `generate_sql_for_import_spec` and `generate_sql_for_export_spec` take `(ctx: *mut c_void, json_spec: *const c_char, result: *mut *mut c_char)`, the shape `virtual_schema_adapter_call` already uses, and the dispatcher threads one `SingleCallContext` down that single path.
- **Alternatives:** Add parallel context-carrying slots beside the existing two, preserving ABI 9. Rejected: two shapes for one concept, and the SDK version bump already forces downstream rebuilds.
- **Rationale:** A hook that builds a `SELECT` needs `script_schema()`, `node_count()`, and `connection(name)`. Extends ADR `abi-version-bump-2-3-vs-adapter-call` from the one vs-adapter slot to every hook that can need host state, fixing the shape for future hooks.
- **Consequences:** `EXA_UDF_ABI_VERSION` goes `9 → 10`. `invoke_vs_adapter_call` generalizes to serve all three hooks.
- **Promotes to ADR:** yes

### [2] `json_spec` is a mechanical 1:1 JSON mirror of the proto message

- **Decision:** Every proto field appears under its proto name. Repeated fields become arrays, nested messages become objects, absent `optional` fields become `null`, `parameters` stays an array of `{"key","value"}` objects, and a `column_type` renders as its variant name.
- **Alternatives:** Collapse `parameters` to a JSON map and omit absent fields. Rejected: the map form is lossy for duplicate keys, and a varying key set forces every author to branch on presence.
- **Rationale:** The mapping is derivable from `zmqcontainer.proto` alone, so a proto change needs no translation table.
- **Promotes to ADR:** no

### [3] Hooks receive the JSON as `&str`, and the SDK parses it behind one optional feature per statement

- **Decision:** The hook signature is `fn(&mut dyn UdfContext, &str) -> Result<String, UdfError>` in every build configuration. The non-default `import` feature adds `ImportSpec::from_json` and the non-default `export` feature adds `ExportSpec::from_json`. Both enable the same two optional dependencies.
- **Alternatives:** Make a `serde`-derived `ImportSpec` / `ExportSpec` the hook parameter. Rejected: a typed parameter puts `serde` in every UDF build and makes the vtable layout depend on a cargo feature. Shipping no typed struct at all was also rejected on user direction, because every author then writes the same parser. One combined `spec-types` feature was rejected on user direction: an author implements import UDFs or export UDFs, so the feature name states which.
- **Rationale:** The feature gate answers the mandatory-dependency objection without rejecting the typed struct. `emit-arrow` already sets this shape for an optional dependency of this crate.
- **Consequences:** `exasol-udf-sdk` gains `serde` and `serde_json` as optional dependencies, and a default build is unchanged. Two items are shared by both features and therefore compile under either one: the `key_value_pair` mirror behind `parameters`, and the `Deserialize` derive on the crate's existing `ConnectionObject`. The column mirror behind `subselect_column_specification` belongs to `import` alone, and `export` adds no type of its own. The structs mirror the pinned JSON shape without normalizing it, so `exa-zmq-protocol` stays the only owner of the proto-to-`ExaType` mapping. The `import-export-spec` fixture enables both features, so the live IMPORT and EXPORT scenarios exercise the typed parse.
- **Promotes to ADR:** no

### [4] Annotation syntax is `import_spec(path)` and `export_spec(path)`

- **Decision:** The two new sections use parenthesized-path syntax.
- **Alternatives:** The `import_spec = path` form the issue's checklist writes. Rejected: `vs_adapter(path)` already owns the path-section syntax, and `name = "..."` is the only key-equals section.
- **Rationale:** The issue's stated deliverable is to wire these "the same way `vs_adapter` wires the VS-adapter hook today".
- **Promotes to ADR:** no

### [5] `input_column_count` is the only name for the input column count

- **Decision:** `input_column_count` is the required trait method. `num_columns` is removed.
- **Alternatives:** Keep `num_columns` as a `#[deprecated]` provided method forwarding to the new name. Rejected on user direction: a project that updates has to use `input_column_count`, so this plan ships no transition alias.
- **Rationale:** Java, Python, and Lua all pair `input_column_count` with `output_column_count`, and the SDK already exposes `output_column_count`.
- **Consequences:** Every `impl UdfContext` and every call site names `input_column_count` or fails to compile. The SDK ships `TestContext` and `DefaultsCtx` so no fixture hand-writes an implementation.
- **Promotes to ADR:** no

### [6] The group row count is named `rows_in_group`

- **Decision:** `UdfContext::rows_in_group() -> u64`.
- **Alternatives:** `size()`, matching Java's `ExaIterator.size()` and the Python and Lua `ctx.size()`. Rejected: `size` names no subject in Rust and collides with the reader's expectation of a byte or element count.
- **Rationale:** The name matches the `exascript_table_data` field it reports and the `node_count` / `output_column_count` accessors beside it.
- **Promotes to ADR:** no

### [7] `MT_UNDEFINED_CALL` reports the SDK hook name

- **Decision:** The dispatcher maps each `SC_FN_*` id to the SDK method an author would implement, and falls back to the protobuf variant name for the `SC_FN_NIL` sentinel.
- **Alternatives:** Keep `fn_id.as_str_name()`. Rejected: the database then raises an error naming a protobuf variant that appears nowhere in the Rust API.
- **Rationale:** The mapping only became possible once both spec hooks gained real SDK methods.
- **Promotes to ADR:** no

### [8] `input_type` and `output_type` accessors are in scope

- **Decision:** `UdfContext::input_type() -> Option<InputType>` and `output_type() -> Option<OutputType>` report the SCALAR/SET and RETURNS/EMITS axes as provided methods defaulting to `None`. `InputType` and `OutputType` are new `exasol-udf-sdk` enums.
- **Alternatives:** Leave both out of scope, because the issue scopes them as "mention only if cheap" and no acceptance criterion names them. Rejected on user direction. Reusing `exa-zmq-protocol`'s single `IterType` enum for both axes was also rejected: `ExactlyOnce` and `Multiple` are the proto's vocabulary, and one enum across two axes makes the author recall which SQL shape each variant means on each side.
- **Rationale:** `exascript_metadata` already carries both axes as `required` fields and the host context already branches on them, so the accessors expose state the runtime holds. Java, Python, and Lua expose the same pair under these names.
- **Consequences:** `HostContextBridge` and `SingleCallContext` report the declared axes. `Option` reports a context with no host metadata, matching `current_user()`, because an enum has no neutral value the engine blesses the way `0` covers `rows_in_group`.
- **Promotes to ADR:** no

### [9] The EXPORT fixture reports through the UDF error channel

- **Decision:** `EXPORT_WORKER` returns its observed-specification summary as a `UdfError`, so the live-DB scenario asserts on the surfaced error text.
- **Alternatives:** Write the summary back over connect-back into an audit table. Rejected: it adds a connect-back dependency to the fixture for a read-back path the existing adapter scenario already solves this way.
- **Rationale:** An `EXPORT` statement discards the generated `SELECT`'s result, so the error channel is the only path back to the client.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Engine population of `rows_in_group` is settled, not an open question

- **Finding:** plan.md and the `runtime/rowset-codec` live scenario treated engine population of `rows_in_group` as an empirical unknown, while `FINDINGS.md:258-262` and `../db/Engine/src/exscript/pluggable/zmqcontainer.cc:415` already record that `create_next_response` always sets the field.
- **Direction change:** The Consequences row and the live scenario state the engine behaviour as fact with those citations. `runtime/rowset-codec`'s Background carries the field's semantics in the citation form the timestamp paragraph already uses.
- **Promotes to ADR:** no

### [plan-review] `call_arg_hook` deletion reconciled with the recorded spec

- **Finding:** The plan deletes `call_arg_hook`, which recorded `specs/runtime/dispatch-single-call/spec.md:52` names in a MUST clause, so the merged library would require a helper the code no longer has.
- **Direction change:** The `runtime/dispatch-single-call` delta carries a `DELTA:CHANGED` block for "Single-call hook error text is surfaced when rc != 0" whose WHEN clause names `call_noarg_hook` and `call_ctx_arg_hook` only.
- **Promotes to ADR:** no

### [plan-review] `rows_in_group` is specified for SCALAR input, for `0`, and after exhaustion

- **Finding:** Three cases were unspecified. The engine reports a vector-chunk size for SCALAR (`FINDINGS.md:260-262`), the proto comments `0` as "no group defined" rather than "not reported", and no step pinned the value after `next()` returns false.
- **Direction change:** The `sdk/udf-sdk` accessor scenario pins all three. The `runtime/rowset-codec` decode scenario pins the SCALAR case, and that feature's Background carries the engine citation.
- **Promotes to ADR:** no

### [plan-review] Variadic runtime schema discovery is asserted live

- **Finding:** Acceptance criterion 9 of issue #45 asks for a live variadic worker reading its schema at runtime. Only a unit test asserted it. The `IMPORT FROM SCRIPT` scenario asserted the hook's `json_spec` read alone.
- **Direction change:** That scenario and task 2.9 require the inserted rows to carry the column count and each declared column name and type read through `ctx.input_column_count()` and `ctx.input_column(idx)`.
- **Promotes to ADR:** no

### [plan-review] The version bump is its own dependency-ordered task

- **Finding:** plan.md § Impact named the workspace version bump, the `exasol-udf-sdk` pin, and the regenerated `Cargo.lock`, but no task performed them. The bump changes `EXA_SDK_FINGERPRINT`, so its position relative to the integration checklist step is load-bearing.
- **Direction change:** Group 4 "Release hygiene" holds task 4.1, and § Parallelization group D depends on A, B and C. The task states the fingerprint consequence and the required `test-udfs/*.so` rebuild.
- **Promotes to ADR:** no

### [plan-review] New fixture crates reach both workspace crate lists

- **Finding:** Tasks 2.8 and 3.2 named the dev-dependency entry and the CI `-p` allowlist but not the root `Cargo.toml` `members` and `default-members` lists, which are explicit enumerations. Without them the local `cargo test -p it` path fails the `dlopen`.
- **Direction change:** Both tasks and both new crate scenarios in `examples/test-udfs` carry the `members`/`default-members`/CI-allowlist requirement in the form recorded at `specs/examples/test-udfs/spec.md:79`. `Cargo.toml` joins the Knowledge column of groups B and C.
- **Promotes to ADR:** no

### [plan-review] Existing `MT_UNDEFINED_CALL` assertions move to the SDK hook names

- **Finding:** Decision 7 renames the reported hook, breaking the assertions at `crates/exa-udf-runtime/tests/single_call.rs:237` and `:1056`, which no task covered.
- **Direction change:** Task 2.6 updates both call sites to `generate_sql_for_export_spec` and `virtual_schema_adapter_call`.
- **Promotes to ADR:** no
