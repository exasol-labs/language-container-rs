# Plan Review Findings: add-import-export-spec-hooks (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 19 (Blockers: 7, Advisory: 12)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Three failure stories drove this pass.

1. The PR lands. Downstream UDF authors rebuild against ABI 10 and get a fingerprint mismatch, because the workspace version never moved. The recorded library still names `call_arg_hook` as a MUST after the code deleted it, so `speq feature validate` and the next plan both read a spec that does not describe the code.
2. `cargo test -p it --features integration` fails on the first run with `reading UDF artifact .../libimport_export_spec.so: No such file or directory`. The two new fixture crates reached the CI allowlist but never reached the workspace `default-members` list, which is what builds the `.so` locally.
3. Six months later a UDF author calls `ctx.rows_in_group()` inside a SCALAR script and sizes a buffer from a vector-chunk count. The spec never said what the accessor means outside a SET group, and `0` reads as both "the SDK default" and "the engine reports no group".

Each story routes into the taxonomy below.

## Intent Fidelity

[no objection - axis checked: traced all nine acceptance criteria of issue #45 against the artifacts. AC1 macro wiring -> task 1.5 plus `sdk/udf-abi` scenario 1. AC2 `UdfRun` hooks and spec reconciliation -> task 1.3 plus the `sdk/udf-sdk` CHANGED scenario, which supersedes the stale claim at recorded `specs/sdk/udf-sdk/spec.md:44`. AC3 serialized `json_spec` -> tasks 2.1 and 2.2 plus the `sdk/udf-sdk` JSON-mirror scenario. AC4 `UdfContext` on both hooks -> `runtime/dispatch-single-call` scenario 2. AC5 `MT_UNDEFINED_CALL` naming -> task 2.4. AC6 `num_columns` rename -> task 1.1. AC7 correcting `crates/exa-udf-runtime/tests/single_call.rs:853-906` -> task 2.6. AC8 live IMPORT and EXPORT -> tasks 2.9 and 2.10. AC9 is covered but under-asserted, raised under Requirement Quality rather than here, because the plan does register `IMPORT_WORKER` variadic and does run it live. The paren annotation syntax and the whole-issue scope were both settled in the clarifying interview and are not re-litigated. The user's standing directive on spec and decision bloat is checked under Design Depth and Prose Quality: exactly one entry carries `Promotes to ADR: yes`, no delta narrates "gap N was X", and no delta restates issue prose]

## Feasibility

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Consequences row 3, and `runtime/rowset-codec/spec.md` § Scenario "The database reports a non-zero group row count over a live connection"
- Issue: the plan treats engine population of `rows_in_group` as an open empirical question. plan.md:54 reads "if the engine leaves the field at its proto default, the accessor is dead on arrival", and the spec step reads "so the scenario fails rather than passes if the database leaves `rows_in_group` at the proto default". Two sources already on disk settle it. `/home/crusty/code/slc-rs/FINDINGS.md:258-262` records the reading: "`create_next_response` (`:402-416`) sets type `MT_RESET` or `MT_NEXT` and always sets `rows_in_group = inp.rowsInGroup()` (`:415`). ... For SET, `rows_in_group` is the full group size (`vmciterators.h:147-150`); for SCALAR it is the current vector-chunk size (`:70-73`)." The engine source itself is present at `/home/crusty/code/db/Engine/src/exscript/pluggable/zmqcontainer.cc:415`, the same tree `specs/runtime/rowset-codec/spec.md` already cites for the timestamp contract, and `/home/crusty/code/db/Engine/src/exscript/pluggable/zmqcontainer.proto:52-54` declares the field `required` with the comment "Rows count in current group in EXASolution / Can be 0 if no group defined". The plan carries an unverified risk into its last task group while the evidence sits in the repo.
- Fix: In plan.md § Consequences, replace row 3 with a statement of fact citing `FINDINGS.md:258-262` and `../db/Engine/src/exscript/pluggable/zmqcontainer.cc:415`. In `runtime/rowset-codec/spec.md`, drop the "fails rather than passes" hedge from the live scenario and state the positive requirement. Add the citation to that feature's `## Background` in the same form the existing timestamp paragraph uses.
- Escalation: MECHANICAL - the fact is settled by a file in this repo and by the reference tree the spec library already cites; no judgment call is needed.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Checklist row "Lint", and § Scenario Coverage row `num_columns_forwards_to_input_column_count`
- Issue: the checklist runs `cargo clippy --all-targets --all-features -- -D warnings`. The scenario `num_columns stays available as a deprecated alias` requires a test that calls the `#[deprecated]` method. That call raises the `deprecated` lint, which `-D warnings` turns into an error. No `#[allow(deprecated)]` exists anywhere in the repository today, so the plan assumes a suppression it never states.
- Fix: Add to task 1.1 the instruction to scope `#[allow(deprecated)]` to the alias test and to any remaining in-repo call site the rename cannot remove.
- Obsolete: decision-log entry 5 now removes `num_columns` outright, so no `#[deprecated]` method and no alias test exist.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Implementation Tasks group 3, and `runtime/rowset-codec/spec.md`
- Issue: two recorded artifacts bear on `rows_in_group` and neither is acknowledged. ADR 022 (`specs/_decision/022-fix-run-dispatch-iteration-type.md:66`) recorded that "`rows_in_group` remains an implementation detail, not the group-boundary mechanism". An archived delta at `specs/_recorded/009-fix-emit-ingest-wire-path/runtime/dispatch-run-loop/spec.md:22` requires that "on the first `MT_NEXT` of a SET group it MUST reserve emit-buffer row capacity from that batch's `rows_in_group`", and that scenario reached neither the library (`specs/runtime/dispatch-run-loop/spec.md` has zero hits) nor the code (`crates/exa-udf-runtime/src/rowset.rs` reads the field nowhere). A reader of the merged spec cannot tell whether task 3.1 reverses ADR 022 or restores plan 009.
- Fix: Add a decision-log entry stating that promoting `rows_in_group` to a read accessor does not reverse ADR 022, because the accessor reports a count and not a group boundary. In task 3.1, instruct the implementer to check whether the plan-009 emit-buffer reservation shipped and to record the answer.

#### [NFR_IGNORED] ADVISORY
- Location: `examples/test-udfs/spec.md` § Scenario "import-export-spec generates IMPORT and EXPORT SQL from the spec payload"
- Issue: `export_specification_rep.connection_information` carries the CONNECTION object's password (`crates/exa-proto/proto/zmqcontainer.proto:141-146`). The `runtime/dispatch-single-call` delta correctly bans the dispatcher from logging the serialized specification, but the fixture scenario then routes a summary of that same payload out through `UdfError`, and UDF error text reaches the database log. The scenario names "the connection name, the parameters, and the column names" without stating that `connection_information` is excluded, so a later fixture edit can leak the password into a durable log.
- Fix: Add a step to that scenario requiring both workers to exclude `connection_information` from the summary they surface, and state that the exclusion holds for the error channel specifically.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: plan.md § Dead Code Removal and task 2.3, against recorded `specs/runtime/dispatch-single-call/spec.md:52`
- Issue: the plan deletes `call_arg_hook`. The recorded scenario "Single-call hook error text is surfaced when rc != 0" names it in a MUST clause: "* *WHEN* the single-call dispatcher invokes the hook via `call_noarg_hook`, `call_arg_hook`, or `call_ctx_arg_hook`". The delta for `runtime/dispatch-single-call` contains no `DELTA:CHANGED` block for that scenario, so after the merge the library requires a helper the code no longer has.
- Fix: Add a `<!-- DELTA:CHANGED -->` block to `specs/_plans/add-import-export-spec-hooks/runtime/dispatch-single-call/spec.md` reproducing the recorded scenario "Single-call hook error text is surfaced when rc != 0" with the WHEN clause reduced to `call_noarg_hook` and `call_ctx_arg_hook`.
- Escalation: MECHANICAL - the conflict is visible by reading the delta against the recorded spec.

#### [COMPLETENESS_GAP] BLOCKER
- Location: `sdk/udf-sdk/spec.md` § Scenario "UdfContext reports the row count of the current input group", and `runtime/rowset-codec/spec.md` § Scenario "InputRowSet carries the group row count of the input batch"
- Issue: three cases are unspecified, and the engine source contradicts one of them. First, SCALAR input: `FINDINGS.md:260-262` records that for SCALAR the engine reports the current vector-chunk size, not a group size, yet no scenario states what `rows_in_group()` returns for a SCALAR UDF. Second, the meaning of `0`: the delta says the default `0` denotes "not reported", while the engine's own proto comments the field "Can be 0 if no group defined" (`../db/Engine/src/exscript/pluggable/zmqcontainer.proto:52-53`), so a caller cannot distinguish an absent host from a non-grouped query. Third, exhaustion: the reference emulator's `size()` returns `0` once the iterator finishes (`../db/Engine/src/exscript/pluggable/emul/library.py:372-375`), while the delta requires the value to "stay constant across every `next()` step of one group". No scenario pins the value after `next()` returns false.
- Fix: In `sdk/udf-sdk/spec.md`, add steps to the `rows_in_group` scenario stating what the accessor returns for SCALAR input and after input exhaustion, and replace the "not reported" gloss with the engine's own meaning for `0`. In `runtime/rowset-codec/spec.md`, add a scenario or steps covering the SCALAR case, citing `FINDINGS.md:258-262`.
- Escalation: MECHANICAL - every open case is answered by the reference source the spec library already cites and by the repository's own FINDINGS record.

#### [COMPLETENESS_GAP] BLOCKER
- Location: `runtime/dispatch-single-call/spec.md` § Scenario "IMPORT FROM SCRIPT runs the generated SQL against its worker UDF", and plan.md § Scenario Coverage
- Issue: acceptance criterion 9 of issue #45 asks for an IT scenario covering a variadic `(...)` worker script reading its schema at runtime. The plan registers `IMPORT_WORKER` variadic and runs it live, but the only scenario that asserts the runtime schema read is `examples/test-udfs: import-export-spec's worker reads a variadic input schema at runtime`, which plan.md:151 maps to a Unit test in `test-udfs/import-export-spec/src/lib_tests.rs`. The IMPORT live scenario asserts only what the hook read out of `json_spec`: the connection name, the `WITH` parameters, `is_subselect`, and the `IMPORT INTO (...)` column names. Nothing live asserts that the worker itself discovered its schema.
- Fix: Add a step to the `IMPORT FROM SCRIPT` scenario requiring the inserted rows to carry the column count and each declared column name and type that `IMPORT_WORKER` read through `ctx.input_column_count()` and `ctx.input_column(idx)` at runtime. Add the matching assertion to task 2.9.
- Escalation: MECHANICAL - the plan already carries the fixture behaviour and the live run, so only the assertion is missing.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: plan.md § Implementation Tasks, task 2.7
- Issue: task 2.7 reads "Update `test-udfs/single-call-fixture` to the three-argument spec slots and echo the received `json_spec`". The fixture wires only the import slot and leaves `generate_sql_for_export_spec` as `None` (`test-udfs/single-call-fixture/src/lib.rs:110-122`). The recorded archive at `specs/_recorded/002-refactor-rowset-dispatch-complexity/review-findings.md:51-52` records that `unimplemented_hook_replies_undefined_call` depends on the export hook being the only `None`, and the delta's own `MT_UNDEFINED_CALL` scenario repeats that premise. The plural wording invites an implementer to wire both.
- Fix: Reword task 2.7 to state that only the import slot changes signature and that `generate_sql_for_export_spec` stays `None`, because `runtime/dispatch-single-call`'s `MT_UNDEFINED_CALL` scenario depends on it.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Implementation Tasks, tasks 1.4 and 2.11
- Issue: acceptance criterion 3 of issue #45 asks for the JSON shape to be "specified in `specs/sdk` and documented on the ABI slot". The shape is pinned in `sdk/udf-sdk/spec.md`, and task 2.11 documents it in `docs/writing-a-udf.md`. No task adds the doc comment on the slot itself in `crates/exasol-udf-sdk/src/abi.rs`, which is where an author reading the vtable looks.
- Fix: Extend task 1.4 to add a doc comment on both spec slots naming the `json_spec` contract and pointing at the `sdk/udf-sdk` scenario that pins it.

## Task Breakdown

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Implementation Tasks, and § Impact bullet 3
- Issue: plan.md:77 states "The release needs a minor `[workspace.package].version` bump, the matching `exasol-udf-sdk` pin in `[workspace.dependencies]`, and a regenerated `Cargo.lock` in the same PR." No task performs it. The only version line in the task list is task 1.4, which bumps `EXA_UDF_ABI_VERSION`, a different constant. The house pattern in every recorded plan that touched this surface is an explicit, separately numbered, dependency-ordered task: `specs/_recorded/008-add-current-user-e2e-tests/plan.md:148`, `specs/_recorded/007-change-slc-runtime-debian/plan.md:190`, `specs/_recorded/002-refactor-rowset-dispatch-complexity/plan.md:132`, `specs/_recorded/003-add-arm64-support/plan.md:99`. Ordering is load-bearing and is the exact defect two earlier reviews raised (`specs/_recorded/002-refactor-rowset-dispatch-complexity/review/round-1.md:54-55`): the bump changes `EXA_SDK_FINGERPRINT`, so every `test-udfs/*.so` must be rebuilt before `cargo test -p it --features integration` runs, and plan.md § Checklist sequences Build, Test, Integration with no bump step anywhere.
- Fix: Add a group 4 "Release hygiene" with one task: bump `[workspace.package].version` to the next minor, update the pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` to match, and commit the regenerated `Cargo.lock`. In § Parallelization, add group D depending on A, B and C, with Knowledge `Cargo.toml`, `Cargo.lock`. State in the task that the bump changes `EXA_SDK_FINGERPRINT`, so every `test-udfs/*.so` must be rebuilt before the integration checklist step runs.
- Escalation: MECHANICAL - the plan's own § Impact names the work, and four recorded plans supply the task template.

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Implementation Tasks, tasks 2.8 and 3.2, and both new scenarios in `examples/test-udfs/spec.md`
- Issue: the two new fixture crates need entries in the root `Cargo.toml` `members` list (`Cargo.toml:3-44`) and `default-members` list (`Cargo.toml:47-81`). Both are explicit enumerations, not globs. Tasks 2.8 and 3.2 name only "its `exa-udf-runtime` dev-dependency entry, and its CI `-p` allowlist line". Without `default-members`, plain `cargo build --release` and the local `cargo test -p it` path that CLAUDE.md documents do not build the `.so`, so the `dlopen` fails locally the same way the CI allowlist gap fails in CI. The recorded library already states the rule per fixture: `specs/examples/test-udfs/spec.md:79` reads "* *AND* the crate MUST appear in the workspace `members` and `default-members` lists and in the CI \"Build UDF .so artifacts (release)\" `-p` allowlist, because an integration scenario `dlopen`s it". Neither new scenario carries that step.
- Fix: Extend tasks 2.8 and 3.2 to add each crate to the root `Cargo.toml` `members` and `default-members` lists. Add the `members`/`default-members`/CI-allowlist step from `specs/examples/test-udfs/spec.md:79` to both new scenarios in `examples/test-udfs/spec.md`. Add `Cargo.toml` to the Knowledge column of groups B and C in § Parallelization.
- Escalation: MECHANICAL - the requirement is already written in the recorded spec and the two lists are visible in `Cargo.toml`.

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Implementation Tasks, tasks 2.4 and 2.6
- Issue: task 2.4 maps every `SC_FN_*` id to its SDK hook name, and the `runtime/dispatch-single-call` delta states the protobuf variant name survives "only for the `SC_FN_NIL` sentinel". Two existing assertions in `crates/exa-udf-runtime/tests/single_call.rs` hold the old strings: line 237 asserts `"SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC"` and line 1056 asserts `"SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL"`. Task 2.6 covers only `import_spec_hook_error_surfaces_as_run_error` plus new cases, so the two tests break with no task accounting for them.
- Fix: Extend task 2.6 to update the existing `MT_UNDEFINED_CALL` assertions at `crates/exa-udf-runtime/tests/single_call.rs:237` and `:1056` to the SDK hook names `generate_sql_for_export_spec` and `virtual_schema_adapter_call`.
- Escalation: MECHANICAL - both call sites are named and the expected new strings follow from the plan's own decision 7.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Parallelization, group B Knowledge column
- Issue: task 2.1 implements the `sdk/udf-sdk` scenario "Spec-generation hooks receive the specification as a JSON mirror of the proto message", which plan.md:133 maps to `crates/exa-udf-runtime/src/spec_json_tests.rs`. Group B's Knowledge lists only the `runtime/dispatch-single-call`, `protocol/single-call` and `examples/test-udfs` deltas, so the agent executing group B does not receive the delta that defines the shape it must encode.
- Fix: Add the `sdk/udf-sdk` delta to group B's Knowledge column.

#### [CLUSTER_INCOHERENCE] ADVISORY
- Location: plan.md § Parallelization
- Issue: the table presents three groups but describes a strictly linear chain. B depends on A, and C depends on A and B, so nothing runs concurrently. Group C is also group A's own feature sliced by layer: task 1.2 adds `UdfContext::rows_in_group()` in group A while tasks 3.1 to 3.3 add the runtime carrier, fixture and live scenario for the same accessor in group C, and C's Knowledge overlaps A on `crates/exa-udf-runtime/src/rowset.rs` and B on `crates/it/tests/db_roundtrip.rs`, `.github/workflows/ci.yml` and the `examples/test-udfs` delta. Separately, task 2.5 tightens `crates/exa-zmq-protocol/src/loop_tests.rs` against the `protocol/single-call` delta, shares no source module or spec delta with the rest of group B, and has no dependency on A, so grouping it into B serializes it needlessly.
- Fix: Move task 1.2 into group C so the `rows_in_group` accessor and its carrier share one group. Move task 2.5 into its own group with no dependencies. State in § Parallelization that the remaining groups run in sequence.

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md § Implementation Tasks, tasks 2.9 and 2.10
- Issue: the five `[expert]` tags sit on tasks 1.1, 1.4, 1.5, 2.1 and 2.2, all of which the plan or the codebase already specifies in detail. Tasks 2.9 and 2.10 carry the plan's real uncertainty and carry no tag. No `IMPORT FROM SCRIPT` or `EXPORT INTO SCRIPT` statement exists anywhere in the repository or the spec library. The `CREATE SCRIPT` shape for a spec-generation script is nowhere in the repo. Every existing CONNECTION object in `crates/it/tests/db_roundtrip.rs` is a connect-back self-connection, never a target a spec hook reads. The existing single-call live scenarios are a load-only smoke test and one that asserts on an expected failure's error text.
- Fix: Tag tasks 2.9 and 2.10 `[expert]`, and name in each the `CREATE SCRIPT` declaration shape and the CONNECTION object the scenario needs.

## Design Depth

[no objection on ADR discipline - axis checked: exactly one entry carries `Promotes to ADR: yes`. Entry 1 changes the `#[repr(C)]` slot signature across the `.so` boundary and forces `EXA_UDF_ABI_VERSION` 9 to 10, so it passes the promotion gate on both behaviour and architecture. Its cited parent, `abi-version-bump-2-3-vs-adapter-call`, resolves as the ADR ID at `specs/_decision/008-2026-06-11-vs-adapter-and-single-call-connect-back.md:5`. The three decisions the brief asked about are correctly left at `no`: entry 3 (raw JSON hook signature) and entry 4 (paren annotation syntax) are API choices that no `.so` fingerprint depends on, and entry 6 (`rows_in_group` naming) is a naming choice. Entries 2, 5, 7, 8 and 9 are likewise implementation, scope and fixture decisions. The `SingleCallContext` reuse avoids a second context type, and `spec_json.rs` keeps the proto out of the SDK crate, so no `[BOUNDARY_VIOLATION]` and no `[SHALLOW_DESIGN]` applies]

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md § Patterns, row "Single owning module for the wire shape"
- Issue: the row claims "one module decides the JSON shape, so no other module encodes that decision". The shape is in fact encoded by every UDF that implements a hook, because decision 3 hands the author a raw `&str` to parse. The plan pins the same shape in `sdk/udf-sdk/spec.md`, in `docs/writing-a-udf.md` (task 2.11), and, once the advisory above is applied, on the ABI slot. The rationale for `&str` over a typed struct is sound, since a typed struct would make `serde` mandatory for every UDF build, but the Patterns claim overstates the containment.
- Fix: Reword the Patterns row to say that one module produces the shape while every consumer parses it, and record in decision 3 that an optional typed view behind a non-default SDK feature stays available as a later step.

#### [TACTICAL_SHORTCUT] ADVISORY
- Location: decision-log.md § Design Decisions entry 5
- Issue: the entry keeps `num_columns` as a `#[deprecated]` forwarding alias and schedules no removal. Issue #45 scopes the alias to "one release". Without a recorded milestone the alias becomes permanent surface on a trait whose method set is part of the ABI fingerprint.
- Fix: Add to entry 5's `Consequences` the release at which `num_columns` is removed, and state that the removal carries its own ABI version bump.
- Obsolete: decision-log entry 5 now removes `num_columns` in this plan, under the same ABI bump, so no later removal needs scheduling.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: `sdk/udf-abi/spec.md` § Scenario "Spec-generation vtable slots take the context pointer, bumping the ABI version"
- Issue: the scenario title and its step narrate a transition rather than state current content. The step reads "`EXA_UDF_ABI_VERSION` MUST be bumped `9 → 10`". A recorded spec states the current requirement, and the plan's own test name asserts the absolute (`spec_slots_take_context_and_abi_version_is_ten`). The recorded spec already carries three such transition absolutes (`specs/sdk/udf-abi/spec.md:17` "(4 → 5)", `:48` unnumbered, `:86` "`6 → 7`"), so a merged fourth leaves the feature asserting four different versions with no line naming the current one. This is the changelog accretion the user's standing directive asks to avoid.
- Fix: Retitle the scenario to drop "bumping the ABI version" and restate the step as "`EXA_UDF_ABI_VERSION` MUST be `10`, so a `.so` built against an earlier ABI fails the loader's version check with a clear `AbiMismatch` error". Keep the `9 → 10` transition and its rationale in decision-log.md entry 1, which is where a change record belongs.

#### [PROSE_BLOAT] ADVISORY
- Location: plan.md § Design, Context and Decision paragraphs
- Issue: two descriptive sentences exceed the 25-word cap and each joins more than one idea. plan.md:11 runs 30 words: "A hook that generates a `SELECT` needs `script_schema()` to qualify the worker script, `node_count()` to size an export shard count, and `connection(name)` to validate a target system before returning SQL." plan.md:18 runs 29 words in its second sentence and chains "which" and "and": "The specification message is protobuf, which is not ABI-stable across the `.so` boundary, so one runtime module serializes it to JSON and that JSON is the author-facing payload."
- Fix: Split plan.md:11 into one sentence per accessor or reduce it to the three accessor names with their purposes in a list. Split plan.md:18's second sentence into two: one stating that protobuf does not cross the `.so` boundary, one stating that `spec_json.rs` produces the author-facing JSON.

[remaining prose checked - no further objection: the new delta text and decision-log entries contain no em dashes, semicolons, contractions, superlatives or hedged modals. The em dashes present in `protocol/single-call/spec.md`, `sdk/udf-abi/spec.md`, `runtime/rowset-codec/spec.md` and the feature-description lines are verbatim carry-over from the recorded specs and are outside this plan's deltas. No delta narrates "gap N was X and we found it by Y", and no delta restates issue prose]
