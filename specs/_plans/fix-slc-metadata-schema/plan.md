# Plan: fix-slc-metadata-schema

## Summary

Correct the committed `build_info/language_definitions.json` to the Exasol database's `language_definitions` v2 metadata schema, so a database that parses it during initialization accepts the container instead of crash-looping. Replace the substring assertion that let the wrong shape ship in v0.23.0 with a path-addressed contract test that runs against both the committed source and the shipped tarball copy.

## Design

### Context

`lc-rust-0.23.0.tar.gz` cannot be installed as a custom SLC in Exasol Personal. The database reads `build_info/language_definitions.json` out of the extracted tarball and validates it during Engine/Nano initialization, then aborts with `Mismatching schema path: #. Mismatching schema keyword: required`. Initialization restart-loops, so the database never starts and the Rust UDF client never runs. The reporter confirmed the failure is neither malformed JSON, nor an interaction with the official `r` SLC, nor intermittent, and that the same schema applies on the DB `master`, `LTS-2025.1`, and `INR-2025.2` branches.

Three forces shape the fix:

1. **The shipped file states the wrong contract.** The database requires a root `language_definitions` array, `parameters` as key/value objects, and a `deprecation` key. The committed file uses a root `languages` array with string `arguments` and no `deprecation`, so it fails the root-level `required` check before any per-definition key is even reached.
2. **The integration matrix cannot see this bug and never will.** The `it` crate registers the SLC with `ALTER SESSION SET SCRIPT_LANGUAGES`, a URL string carrying `?lang=rust`, which never reads `build_info/language_definitions.json`. The whole `8.29.x / 2025.1.x / 2026.1.x` matrix therefore stays green while the metadata is unusable on the custom-SLC install path. Regression cover must be static, not behavioral.
3. **The existing guardrail could not detect a renamed key.** `slc_tarball_language_definitions_well_formed` in `dist/tests/slc_tarball_test.sh` matches six substrings against the whitespace-stripped file. It asserts `"arguments":["lang=rust"]` but never asserts the root key at all, so `languages` passed it. Restoring confidence needs a structural check, not a corrected substring list.

- **Goals** — ship metadata that conforms to the DB SLC v2 schema; give the schema exactly one owner in the repository; make the check fail on a renamed key or a re-typed field; run that check without a Docker build so a developer sees the failure in seconds.
- **Non-Goals** — no change to the `SCRIPT_LANGUAGES` registration string, `scripts/lib/script_languages.sh`, or `scripts/install.sh`, because the BucketFS registration path does not read this file; no integration test that boots a database with the tarball mounted at `/exa/slc/<name>` and asserts a clean init, because the local container matrix does not exercise the custom-SLC install path at all (filed as follow-up, see Dependencies); no committed JSON Schema document plus a schema-validator tool; no change to the database itself, which is outside this repository.

### Decision

Fix the document, then move the schema knowledge into one script that takes the document as an argument, so both the pre-build source check and the post-build artifact check read the same encoding of the contract.

#### Architecture

```
build_info/language_definitions.json          ← the one committed document
              │
              │ subject of
              ▼
dist/tests/language_definitions_test.sh <file>   ← sole owner of the DB SLC v2 contract
              ▲                        ▲
   invoked on the committed     invoked on the extracted
   source, no Docker needed     tarball copy
              │                        │
   .github/workflows/ci.yml     dist/tests/slc_tarball_test.sh
   (unit-tests job)                    │
                                       ├─ shipped copy == committed source (cmp)
                                       └─ declared executable resolves in the tree
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| One committed artifact owns a contract, every checker reads it | `dist/tests/language_definitions_test.sh` | Mirrors `crates/cargo-exasol-udf/slc-library-surface.txt`, already the single owner of the staged library surface for the Dockerfile, the CLI, and the tarball test. Source and shipped copy cannot state different schemas. |
| Path-addressed assertions with `jq` | same | Substring matching over compacted JSON is blind to a renamed root key, which is exactly how this bug shipped. `jq` addresses `.language_definitions[0].parameters[0].key` directly. |
| Subject passed as an argument | same | One encoding of the schema serves two subjects: the committed source in the cheap CI job, and the extracted artifact in the tarball test. |
| Delegation, not duplication | `dist/tests/slc_tarball_test.sh` | The tarball test keeps only the assertions that need the extracted tree (copy identity, declared executable resolves) and shells out for the shape. |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Replace `arguments` with `parameters`; do not keep both | Keep `arguments` alongside `parameters` for older readers | Every DB branch named in the issue wants `parameters`. Keeping both ships two encodings of the single fact `lang=rust` with nothing enforcing agreement, which is the drift hazard the spec now forbids. |
| Retain `language_identifier` | Drop it as unconsumed | The issue states the field may be retained and that this parser ignores it. It duplicates no other key, so it carries no drift risk, and removing it in a bug fix would change the published artifact for no verified gain. |
| One `jq`-based bash script owns the schema | A Rust `#[test]` deserializing into a strict struct; a committed JSON Schema plus a validator tool | `dist/tests/` already holds three plain-assertion bash contract tests over static config and artifacts, and the `unit-tests` job already runs two of them. A Rust test cannot be reached from `slc_tarball_test.sh` without a second encoding, and `cargo-exasol-udf` is published to crates.io, so it cannot `include_str!` a repo-root file. A JSON Schema document would add a validator tool for a six-field document without adding coverage. |
| Require `jq`, guarded explicitly | Hand-rolled `sed`/`awk` JSON addressing; a host Python validator | `jq` is preinstalled on the GitHub Ubuntu runners this workflow uses and is packaged everywhere; the script already `die`s on a missing `readelf`, so one more named guard is in pattern. Hand-rolled JSON parsing reintroduces the fragility being removed, and the project avoids host Python around the SLC. |
| Verify the Personal custom-SLC install manually | Automate it in `it` | The custom-SLC path needs a database restart with `/exa/slc/<name>` populated, which the local Docker matrix does not do. `container/personal-install` already documents manual verification for Personal, so this follows the established route. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/slim-image | CHANGED | `container/slim-image/spec.md` |

## Impact

Operators can install the released tarball as a custom SLC on an Exasol database without initialization aborting. The metadata file changes shape: the root key becomes `language_definitions`, `arguments: ["lang=rust"]` becomes `parameters: [{"key": "lang", "value": "rust"}]`, and `deprecation: null` is added. Nothing breaks on the BucketFS registration path that `scripts/install.sh` drives, because it reads the `SCRIPT_LANGUAGES` URL string and never this file, so existing `ALTER SYSTEM SET SCRIPT_LANGUAGES` values keep working unchanged. Any external consumer reading `languages` or `arguments` out of the tarball must switch to the new keys; this repository has no such consumer, and `grep` finds the file referenced only by the Dockerfile `COPY` and the tarball contract test.

## Requirements

| Requirement | Details |
|-------------|---------|
| Release artifacts carry DB-conformant metadata | The tarball's `build_info/language_definitions.json` satisfies the DB SLC v2 schema quoted in issue #90 |
| Automated validation of the metadata contract | A test asserts the document against that schema by key path, on every pull request, without a live database |
| Custom-SLC install verified on Exasol Personal | The database reaches a running state with no `Mismatching schema path` line and no InitProcess restart loop |

## Dependencies

- `jq` on any machine running the contract test. Preinstalled on the `ubuntu-latest` and `ubuntu-24.04-arm` runners; the script guards for it and names the missing package.
- `.github/workflows/ci.yml` § `release` auto-tags and publishes `lc-rust-<version>.tar.gz` on merge to `main` once the version is bumped. CLAUDE.md § Build & release requires that bump on every change, and `/speq:implement-pr` performs it. The Personal custom-SLC install check therefore MUST pass before merge, not after — merging publishes the artifact. Shipping first and discovering the schema afterwards is exactly what produced issue #90.
- Follow-up, not part of this plan: integration cover of the custom-SLC install path (extract the tarball into a container's `/exa/slc/<name>`, restart the database, assert a clean init). Track it as a `feature`-labelled GitHub issue, per CLAUDE.md § Specs & issues, so the Non-Goal above stays visible.

## Migration

| Current | New |
|---------|-----|
| `"languages": [ … ]` | `"language_definitions": [ … ]` |
| `"arguments": ["lang=rust"]` | `"parameters": [{"key": "lang", "value": "rust"}]` |
| no `deprecation` key | `"deprecation": null` |
| `"schema_version": 2` | unchanged |
| `"aliases": ["RUST"]`, `"protocol": "localzmq+protobuf"`, `"language_identifier": "rust"`, `"udf_client_path": {"executable": "/exaudf/exaudfclient"}` | unchanged |

## Implementation Tasks

1. Add `dist/tests/language_definitions_test.sh`, in the plain-assertion style of `dist/tests/about_toml_test.sh` (`fail` / `pass` / `die` helpers, a failure counter, non-zero exit when it is non-zero). It takes the document path as its single argument, `die`s on a missing argument, a missing file, or a missing `jq`, and exposes four assertions addressed by `jq` key path: `language_definitions_declares_schema_version_2`, `language_definitions_root_key_is_language_definitions` (which also fails on a present `languages` key), `language_definitions_holds_one_conforming_definition` (`aliases`, `protocol`, `parameters` as one `{key, value}` object, `udf_client_path.executable`, and `deprecation`, whose presence MUST be asserted with `jq -e '.language_definitions[0] | has("deprecation")'` and whose value MUST be separately asserted to be `null`, because `jq` renders an absent key and a null value identically), and `language_definitions_definition_has_no_arguments_key`. Run it against the unchanged `build_info/language_definitions.json` and confirm it fails on the root key. The per-definition failures on that document are not independent evidence: its root key is `languages`, so `.language_definitions[0]` is `null` and every per-definition assertion fails for that one shared reason. Task 2's fixtures supply the per-assertion evidence.
2. Add one-per-defect negative fixtures under `dist/tests/fixtures/language_definitions/`. Each is an otherwise-conforming document varying exactly one fact: `root_key_renamed.json` (root key `languages`), `deprecation_removed.json` (no `deprecation` key), `arguments_readded.json` (`arguments: ["lang=rust"]` present), `parameters_retyped.json` (`parameters: ["lang=rust"]`), `executable_changed.json` (a different `udf_client_path.executable`). Add `conforming.json` alongside them. Then add `dist/tests/language_definitions_fixtures_test.sh`, a driver in the same plain-assertion style, which runs task 1's script on each fixture. It MUST assert a non-zero exit on all five negative fixtures and exit 0 on `conforming.json`. The driver owns these assertions, not the contract script, so the contract script keeps exactly one subject. This task is the plan's only evidence that each assertion discriminates; without it, an assertion that can never fail looks as verified as one that works.
3. Correct `build_info/language_definitions.json` per the Migration table. Re-run task 1's script against it and confirm every assertion passes.
4. Rewrite `slc_tarball_language_definitions_well_formed` in `dist/tests/slc_tarball_test.sh`: delete the `expected` substring array and the `compact` derivation, delegate the shape check to `bash "$HERE/language_definitions_test.sh" "$TREE/build_info/language_definitions.json"` and surface the child's output only when it exits non-zero, add a `cmp` proving the shipped copy is byte-identical to `$ROOT/build_info/language_definitions.json`, and keep the cross-check that the declared `udf_client_path.executable` resolves to an executable file inside the extracted tree.
5. Add a `Run language definitions contract tests` step to the `unit-tests` job in `.github/workflows/ci.yml`, beside the existing `Run about.toml tests` and `Run install-script tests` steps, invoking `bash dist/tests/language_definitions_test.sh build_info/language_definitions.json` and then `bash dist/tests/language_definitions_fixtures_test.sh`.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 |
| Group B | Task 2, Task 3, Task 5 |
| Group C | Task 4 |

Sequential dependencies:
- Group A → Group B (all three tasks need the contract script to exist; task 3 needs its failure first, per failing-test-first)
- Group B → Group C (the tarball test delegates to the script and passes only once the document is corrected)

Task 2 and task 3 touch disjoint files — task 2 writes only under `dist/tests/`, task 3 only `build_info/language_definitions.json` — so they carry no ordering constraint between them.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Assertion body | `dist/tests/slc_tarball_test.sh` — the `expected` substring array and `compact` variable in `slc_tarball_language_definitions_well_formed` | Substring matching cannot see a renamed root key; replaced by path-addressed assertions in `dist/tests/language_definitions_test.sh` |
| JSON keys | `build_info/language_definitions.json` — `languages`, `arguments` | Rejected by the DB SLC v2 schema; replaced by `language_definitions` and `parameters` |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Language definitions file is present and well-formed | Integration | `dist/tests/language_definitions_test.sh`, run on `build_info/language_definitions.json` | `language_definitions_document_is_valid_json`, `language_definitions_declares_schema_version_2`, `language_definitions_root_key_is_language_definitions`, `language_definitions_holds_one_conforming_definition`, `language_definitions_definition_has_no_arguments_key` |
| Language definitions file is present and well-formed | Integration | `dist/tests/slc_tarball_test.sh`, run on the extracted tarball | `slc_tarball_language_definitions_well_formed` |
| Language definition contract is asserted by key path on both copies | Integration | `dist/tests/language_definitions_fixtures_test.sh`; `dist/tests/slc_tarball_test.sh` | `language_definitions_rejects_root_key_renamed`, `language_definitions_rejects_deprecation_removed`, `language_definitions_rejects_arguments_readded`, `language_definitions_rejects_parameters_retyped`, `language_definitions_rejects_executable_changed`, `language_definitions_rejects_schema_version_wrong`, `language_definitions_rejects_definitions_not_array`, `language_definitions_rejects_two_definitions`, `language_definitions_rejects_aliases_changed`, `language_definitions_rejects_protocol_changed`, `language_definitions_rejects_deprecation_non_null`, `language_definitions_rejects_malformed`, `language_definitions_accepts_conforming`; plus `slc_tarball_language_definitions_well_formed` for the byte-identity clause |

The first two rows cover the CHANGED scenario on both copies of the document: the first proves the shape without a Docker build, the second proves the shipped tarball carries that shape. The third row covers the NEW scenario. Its first clause, that each required field's absence, removal, or re-typing fails the check, is proven by twelve fixture cases plus one conforming case, closing every assertion branch the contract script exposes (a code-review pass found the original six left six branches, including one unreachable branch, without evidence). Its third clause, that the contract is checkable without building the image, is proven by the same scripts passing in the `unit-tests` job, which builds no container.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/slim-image | `bash dist/tests/language_definitions_test.sh build_info/language_definitions.json` | Five `PASS:` lines, `All tests passed`, exit 0 |
| container/slim-image | `bash dist/tests/language_definitions_fixtures_test.sh` | Thirteen `PASS:` lines — one per fixture, twelve rejected and one accepted — `All tests passed`, exit 0 |
| container/slim-image | `bash dist/generate-licenses.sh && mkdir -p /tmp/lc-out && docker build --target artifact --output type=local,dest=/tmp/lc-out . && bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` | `All tests passed`, including `PASS: slc_tarball_language_definitions_well_formed` |
| container/slim-image | `tar -xzOf /tmp/lc-out/lc-rs.tar.gz build_info/language_definitions.json \| jq .` | Root key `language_definitions`, one definition with `parameters: [{"key": "lang", "value": "rust"}]` and `deprecation: null`, no `arguments` key |
| container/slim-image | Install `/tmp/lc-out/lc-rs.tar.gz` as a custom SLC on an Exasol Personal deployment, then read the database initialization log | The database reaches a running state; the log holds no `Mismatching schema path` line and no InitProcess restart loop |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Shell contract tests | `bash dist/tests/language_definitions_test.sh build_info/language_definitions.json && bash dist/tests/language_definitions_fixtures_test.sh && bash dist/tests/about_toml_test.sh && bash scripts/tests/install-personal-test.sh` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures (unaffected by this change: `it` registers via `SCRIPT_LANGUAGES` and never reads this file) |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Personal custom-SLC install | install the PR run's `lc-tarball-x86_64` artifact as a custom SLC on an Exasol Personal deployment and read the DB initialization log | database reaches a running state; no `Mismatching schema path` line, no InitProcess restart loop |
