# Decision Log: fix-slc-metadata-schema

## Interview

Headless run, no live interview. GitHub issue exasol-labs/language-container-rs#90 was passed as the authoritative, fully specified input: it carries the failing error text, the root cause, the exact expected metadata shape, and three acceptance criteria. The issue has no comments, so no further exchange exists to record. The reporter's follow-up confirmations were folded into the brief and are recorded here as the interview substitute.

**Q:** Is the released file itself malformed, or is the mismatch against the schema the DB expects?
**A:** The file fetched straight from the release tarball is a well-formed `schema_version: 2` document. The mismatch is against the schema the DB image now expects.

**Q:** Is this an interaction between the official `r` SLC and the custom Rust SLC?
**A:** No. With the official `r` SLC removed, the identical crash occurs with the Rust SLC alone.

**Q:** Is the failure intermittent, or a launcher-level timeout?
**A:** Neither. It crash-loops consistently, verified in the database container's own InitProcess logs rather than at launcher level.

**Q:** Must `language_identifier` be removed?
**A:** It may be retained as an additional field if useful. This DB parser does not consume it.

## Design Decisions

### [1] Ship DB-conformant SLC metadata, validated structurally by a single owning script

- **Decision:** Correct `build_info/language_definitions.json` to the DB SLC v2 schema (root key `language_definitions`, `parameters` as key/value objects, `deprecation` present), and move the schema encoding into one new script, `dist/tests/language_definitions_test.sh`, that takes the document path as an argument. The `unit-tests` CI job runs it on the committed source; `dist/tests/slc_tarball_test.sh` delegates to it for the extracted tarball copy and additionally proves the two copies are byte-identical.
- **Alternatives:** (a) Correct the substring list in the existing tarball assertion. Rejected: substring matching over compacted JSON is what allowed the root key `languages` to ship, because the assertion never addressed the root key at all. (b) A Rust `#[test]` deserializing the document into a strict struct. Rejected: it cannot be reached from `slc_tarball_test.sh` without encoding the schema a second time, and `cargo-exasol-udf`, the crate that already owns the committed SLC contract files, is published to crates.io and so cannot `include_str!` a repository-root file. (c) A committed JSON Schema document plus a schema-validator tool. Rejected: it adds tooling without adding coverage for a six-field document. (d) Assert the shape only on the shipped tarball. Rejected: that check needs a full Docker build, so a developer sees a broken document minutes later instead of seconds later.
- **Rationale:** The bug is a contract stated in two places with nothing reconciling them: the shipped document and the assertion meant to guard it. One script that owns the schema and accepts the subject as an argument removes the second encoding, so the pre-build check and the artifact check cannot disagree. It follows the pattern already established by `crates/cargo-exasol-udf/slc-library-surface.txt`, which the Dockerfile staging loop, `cargo exasol-udf validate`, and the tarball test all read. Bash with `jq` keeps it beside the three existing `dist/tests/*_test.sh` contract scripts, two of which the `unit-tests` job already runs.
- **Promotes to ADR:** yes

### [2] Replace `arguments` with `parameters` rather than keeping both

- **Decision:** Remove the `arguments` key. `parameters: [{"key": "lang", "value": "rust"}]` becomes the only representation of the `lang=rust` launch parameter, and the spec forbids `arguments` from returning.
- **Alternatives:** Keep `arguments` alongside `parameters` for any older reader. The issue's confirmation that a retained extra field is tolerated implies the schema permits it.
- **Rationale:** Every DB branch named in the issue (`master`, `LTS-2025.1`, `INR-2025.2`) requires `parameters`, so `arguments` serves no reader. Two keys encoding the single fact `lang=rust` is a drift hazard with nothing enforcing agreement between them.
- **Promotes to ADR:** no

### [3] Retain `language_identifier`

- **Decision:** Keep `language_identifier: "rust"` in the definition.
- **Alternatives:** Drop it, since this DB parser does not consume it.
- **Rationale:** The issue explicitly permits retaining it. It duplicates no other key, so unlike `arguments` it carries no drift risk, and removing it would change the published artifact for an unverified consumer set with no gain.
- **Promotes to ADR:** no

### [4] Require `jq`, guarded by name

- **Decision:** Address the document with `jq` key paths and `die` with a named guard when `jq` is absent.
- **Alternatives:** Hand-rolled `sed`/`awk` JSON addressing; a host Python validator.
- **Rationale:** `jq` is preinstalled on the `ubuntu-latest` and `ubuntu-24.04-arm` runners this workflow uses, and `slc_tarball_test.sh` already `die`s on a missing `readelf`, so a named tool guard is in pattern. Hand-rolled JSON parsing would reintroduce the fragility being removed, and the project deliberately keeps host Python away from the SLC.
- **Promotes to ADR:** no

### [5] Verify the Personal custom-SLC install manually; do not automate it in this plan

- **Decision:** Cover the third acceptance criterion, a clean database start after a custom-SLC install, as a manual verification step in plan.md. File a follow-up issue for integration cover of that path.
- **Alternatives:** Add an `it` scenario that extracts the tarball into a container's `/exa/slc/<name>`, restarts the database, and asserts a clean init.
- **Rationale:** The local Docker matrix does not exercise the custom-SLC install path at all, and `container/personal-install` already documents manual verification for Personal because Personal is not exercisable in CI. Automating a DB restart with a populated `/exa/slc` is a separate piece of harness work, not part of a metadata fix. Recording it as a tracked follow-up keeps the gap visible instead of silently accepted.
- **Promotes to ADR:** no

### [6] Note why the green integration matrix did not catch this

- **Decision:** Record in plan.md § Design § Context that `it` registers the SLC through `ALTER SESSION SET SCRIPT_LANGUAGES`, a URL string that never reads `build_info/language_definitions.json`, so the `8.29.x / 2025.1.x / 2026.1.x` matrix can stay green while the metadata is unusable on the custom-SLC install path.
- **Alternatives:** Leave it out as background noise.
- **Rationale:** Without this, the obvious reading of a green matrix is that the metadata works on every supported DB version, and the next planner would look for a version-specific cause. It also settles why the regression cover must be a static contract test rather than a behavioral one.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Personal install check must gate the merge, because merging publishes the release

- **Finding:** `plan-reviewer` round 1 flagged a hidden dependency on the repository's auto-release. `.github/workflows/ci.yml` § `release` auto-tags and publishes `lc-rust-<version>.tar.gz` on a green merge to `main` once the version is bumped, and CLAUDE.md § Build & release requires that bump on every change. The manual Personal custom-SLC install is the plan's only proof that the transcribed schema is the schema the database actually compiles, yet it appeared only in § Verification § Manual Testing — absent from the § Checklist that gates implementation, and unnamed in § Dependencies. Merging would therefore publish the artifact before anyone confirmed a database starts with it: the same ship-then-discover sequence that produced issue #90.
- **Direction change:** Added a `Personal custom-SLC install` row to plan.md § Verification § Checklist, verifying the PR run's `lc-tarball-x86_64` artifact against a running Exasol Personal deployment with no `Mismatching schema path` line and no InitProcess restart loop. Added a § Dependencies bullet naming the auto-release ordering constraint: the check MUST pass before merge, not after.
- **Promotes to ADR:** no

### [plan-review] `jq` renders an absent key and a null value identically

- **Finding:** `plan-reviewer` round 1 flagged that task 1 stated the `deprecation` requirement correctly but assumed `jq` path addressing can express it. It cannot: `jq -r '.language_definitions[0].deprecation'` prints `null` both for `"deprecation": null` and for a document with no `deprecation` key, and `jq -e '… .deprecation == null'` is true in both cases. An implementer taking the obvious route would write an assertion that passes on a file missing the one key the database's root `required` check is about — reproducing, on the highest-value field, the absent-versus-expected blindness that let `languages` ship past the substring assertion. The review also showed task 1's negative run was confounded: with root key `languages`, `.language_definitions[0]` is `null`, so every per-definition assertion fails for one shared reason.
- **Direction change:** plan.md § Implementation Tasks task 1 now requires `deprecation`'s presence to be asserted with `jq -e '.language_definitions[0] | has("deprecation")'` and its value to be asserted separately as `null`, stating why. The same task's closing sentence no longer claims the unchanged document yields independent per-assertion failures; it names the shared `null` cause and defers per-assertion evidence to task 2.
- **Promotes to ADR:** no

### [plan-review] No test demonstrated that any assertion discriminates

- **Finding:** `plan-reviewer` round 1 flagged that nothing in the plan proved the assertions can fail. The spec's NEW clause 1 required that "a renamed root key or a re-typed field fails the check", but § Verification § Scenario Coverage row 3 mapped that clause to the checking script itself, which is circular. The plan's only negative evidence was task 1's run against the unchanged document, which is confounded for the reason above and additionally errors — `null | has("arguments")` exits 5 with "null (null) has no keys". An assertion that can never fail would look as verified as one that works. The clause was also untestable as written: "asserted by its key path rather than by substring match" names an implementation technique, not an observable outcome.
- **Direction change:** Inserted a new task 2 in plan.md § Implementation Tasks creating one-per-defect negative fixtures under `dist/tests/fixtures/language_definitions/` — root key renamed, `deprecation` removed, `arguments` re-added, `parameters` re-typed, `udf_client_path.executable` changed — plus a conforming fixture, driven by a new `dist/tests/language_definitions_fixtures_test.sh` that asserts non-zero exit on each negative fixture and exit 0 on the conforming one. The driver owns those assertions so the contract script keeps one subject. Renumbered the following tasks to 3, 4, 5 and updated § Parallelization accordingly; extended task 5's CI step and the § Checklist shell-contract-test command to run the driver. Reworded the spec's NEW clause 1 to the observable form "each required field's absence, removal, or re-typing MUST fail the check", and replaced § Scenario Coverage row 3's test-name cell with the six fixture cases.
- **Promotes to ADR:** yes
