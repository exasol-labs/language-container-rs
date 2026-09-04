# Decisions: fix-slc-metadata-schema

## ADR: Ship DB-conformant SLC metadata, validated structurally by a single owning script

**ID:** language-definitions-single-owning-script
**Plan:** fix-slc-metadata-schema
**Status:** Accepted

### Context

`lc-rust-0.23.0.tar.gz` could not be installed as a custom SLC in Exasol Personal: the database parses and schema-validates `build_info/language_definitions.json` during Engine/Nano initialization, and aborted with `Mismatching schema path: #. Mismatching schema keyword: required`. The committed file stated a `languages`-rooted shape with string `arguments` and no `deprecation` key, while the database's SLC v2 schema requires a root `language_definitions` array, `parameters` as key/value objects, and a `deprecation` key. The existing guardrail, `slc_tarball_language_definitions_well_formed`, matched six substrings against the whitespace-stripped file and never asserted the root key at all, so the renamed key shipped undetected. The local integration matrix cannot see this class of bug either: it registers the SLC via `ALTER SESSION SET SCRIPT_LANGUAGES`, which never reads this file.

### Decision

Correct `build_info/language_definitions.json` to the DB SLC v2 schema (root key `language_definitions`, `parameters` as key/value objects, `deprecation` present), and move the schema encoding into one new script, `dist/tests/language_definitions_test.sh`, that takes the document path as an argument. The `unit-tests` CI job runs it on the committed source; `dist/tests/slc_tarball_test.sh` delegates to it for the extracted tarball copy and additionally proves the two copies are byte-identical.

### Options Considered

| Option | Verdict |
|--------|---------|
| One `jq`-based script, addressed by key path, taking the subject document as an argument | ✓ Chosen — removes the second encoding of the contract; the pre-build check and the artifact check read the same schema and cannot disagree |
| Correct the substring list in the existing tarball assertion | ✗ Rejected — substring matching over compacted JSON is exactly what let the root key `languages` ship undetected, because the assertion never addressed the root key |
| A Rust `#[test]` deserializing into a strict struct | ✗ Rejected — unreachable from `slc_tarball_test.sh` without a second schema encoding; `cargo-exasol-udf` is published to crates.io and cannot `include_str!` a repo-root file |
| A committed JSON Schema document plus a schema-validator tool | ✗ Rejected — adds a validator tool for a six-field document without adding coverage |
| Assert the shape only on the shipped tarball | ✗ Rejected — needs a full Docker build, so a wrong shape surfaces minutes later instead of seconds later |

### Consequences

A single script now owns the DB SLC v2 contract, following the pattern already established by `crates/cargo-exasol-udf/slc-library-surface.txt`. Both the committed source and the shipped tarball copy are checked against the same encoding, so they cannot silently diverge. Adding or changing a required field means editing one script rather than reconciling two. The check depends on `jq`, guarded by name, matching the existing pattern of `slc_tarball_test.sh`'s guard on `readelf`.

## ADR: Key-path assertions must be proven to discriminate, not just to pass

**ID:** language-definitions-fixture-proven-assertions
**Plan:** fix-slc-metadata-schema
**Status:** Accepted

### Context

Plan review found that the initial task design could not demonstrate its own assertions actually fail on a defect. `jq` renders an absent key and an explicit `null` value identically, so a naive check for `deprecation == null` passes both on a conforming document and on one missing the key entirely — reproducing, on the highest-value field, the exact absent-versus-expected blindness that let the `languages` root key ship past the old substring assertion. The only negative run available, against the unchanged pre-fix document, was confounded: because its root key is `languages`, `.language_definitions[0]` evaluates to `null`, so every per-definition assertion failed for one shared reason rather than for its own.

### Decision

Require `deprecation`'s presence to be asserted separately from its value (`jq -e '.language_definitions[0] | has("deprecation")'` for presence, a distinct assertion for the `null` value). Add one-per-defect negative fixtures under `dist/tests/fixtures/language_definitions/` — root key renamed, `deprecation` removed, `arguments` re-added, `parameters` re-typed, `udf_client_path.executable` changed — plus one conforming fixture, driven by `dist/tests/language_definitions_fixtures_test.sh`, which asserts non-zero exit on every negative fixture and exit 0 on the conforming one.

### Options Considered

| Option | Verdict |
|--------|---------|
| One-per-defect negative fixtures plus a driver script that asserts each fails independently | ✓ Chosen — the only way to show each assertion discriminates rather than merely runs |
| Rely on the unchanged pre-fix document as the negative case | ✗ Rejected — confounded: its root key mismatch makes every per-definition assertion fail for one shared reason, proving nothing about the other assertions individually |
| Trust that key-path assertions are self-evidently correct without fixture proof | ✗ Rejected — `jq`'s identical rendering of an absent key and a `null` value is exactly the kind of gap that shipped the original defect |

### Consequences

The contract script and its fixture-driven proof are two separate artifacts with one responsibility each: the script states the schema, the fixtures prove each stated requirement can fail. Extending the schema now carries an explicit obligation to add a matching negative fixture, keeping the discriminating-power guarantee from eroding as the contract grows.
