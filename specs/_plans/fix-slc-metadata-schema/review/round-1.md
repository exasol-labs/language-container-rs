# Plan Review Findings: fix-slc-metadata-schema (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 10 (Blockers: 3, Advisory: 7)
- Intent Fidelity blockers: 0

## Premortem

Three failure stories, written as if this plan shipped and failed.

1. **v0.24.0 crash-loops too.** The corrected document conforms to the schema transcribed from issue #90, not to the schema the database actually compiles. One detail is off — most plausibly `additionalProperties: false` rejecting the retained `language_identifier`, or `parameters` wanting an object map rather than an array. CI is green on all four legs, the version bump auto-tags on merge, `lc-rust-0.24.0.tar.gz` is published, and the reporter reinstalls into the same restart loop. Nobody ran the manual Personal install before the release, because nothing in the plan sequenced it before merge. → Feasibility F1, F3.
2. **The new guardrail is as blind as the old one.** Six months later a Dockerfile change templates `build_info/` and drops `deprecation`. `dist/tests/language_definitions_test.sh` passes, because `jq '.language_definitions[0].deprecation'` prints `null` for an absent key exactly as it does for a present null. The regression reaches release through the same class of defect this plan exists to remove. → Feasibility F2, Requirement Quality R1.
3. **`lang=rust` drifts across two files.** The registration string in `scripts/lib/script_languages.sh` still carries `?lang=rust`; the metadata now carries `{"key": "lang", "value": "rust"}`. Someone renames the launch parameter in one file. The BucketFS path keeps working, the custom-SLC path breaks, and no test compares the two. → Design Depth D1.

## Intent Fidelity

[no objection — axis checked]

- AC1 (release artifacts carry conformant metadata) is structurally closed: `.github/workflows/ci.yml` § `release` publishes `lc-rust-${VER}.tar.gz` copied from the `lc-tarball-x86_64` artifact that `build-slc` produced and `slc_tarball_test.sh` already gated, and `Dockerfile:126` (`COPY build_info/ /slc/build_info/`) copies the committed file verbatim.
- AC2 (automated validation) is closed by tasks 1, 3 and 4.
- AC3 (Personal install verified) is retained, not dropped — plan.md § Requirements row 3 and § Verification § Manual Testing row 4 both carry it. Its defect is a missing ordering gate, raised under Feasibility F1 rather than here.
- The Migration table produces the issue's expected document verbatim, plus the retained `language_identifier` that the issue's own text authorizes. No substituted problem, no gold-plating: the four tasks touch one JSON document, two shell scripts and one CI step.

## Feasibility

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: plan.md § Verification § Checklist and § Dependencies; decision-log.md § [5]
- Issue: this repository auto-releases. `.github/workflows/ci.yml` § `release` states "auto-tag + publish on merge to main when the version is bumped" — on a green push to `main` it tags, cuts the GitHub Release with `files: lc-rust-*.tar.gz`, and publishes the crates. CLAUDE.md § Build & release requires a version bump on every change, and `/speq:implement-pr` performs it. The manual Personal install is the plan's only validation that the transcribed schema is the real one, yet it appears only in § Manual Testing, absent from the § Checklist that gates implementation, and § Dependencies never names the ordering constraint. Merging therefore publishes `lc-rust-<next>.tar.gz` before anyone has confirmed a database starts with it — the exact ship-then-discover sequence that produced #90.
- Fix: Add a row to plan.md § Verification § Checklist reading `| Personal custom-SLC install | install the PR run's `lc-tarball-x86_64` artifact as a custom SLC on an Exasol Personal deployment and read the DB initialization log | database reaches a running state; no `Mismatching schema path` line, no InitProcess restart loop |`, and add a bullet to plan.md § Dependencies stating that `.github/workflows/ci.yml` § `release` publishes on merge to `main` once the version is bumped, so this check MUST pass before merge, not after.

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Implementation Tasks task 1 — "a `deprecation` key present with value `null`"
- Issue: the plan states the requirement correctly but assumes `jq` path addressing can express it. It cannot. `jq -r '.language_definitions[0].deprecation'` prints `null` both for `"deprecation": null` and for a document with no `deprecation` key at all, and `jq -e '... .deprecation == null'` is true in both cases. An implementer taking the obvious route writes an assertion that passes on a file missing the one key the database's root `required` check is about. That reproduces, on the highest-value field, the same absent-versus-expected blindness that let `languages` ship past the substring assertion.
- Fix: In plan.md § Implementation Tasks task 1, replace "and a `deprecation` key present with value `null`" with "and `deprecation`, whose presence MUST be asserted with `jq -e '.language_definitions[0] | has(\"deprecation\")'` and whose value MUST be separately asserted to be `null`, because `jq` renders an absent key and a null value identically".

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Design § Consequences row 2 ("Retain `language_identifier`"); decision-log.md § [3]
- Issue: retaining `language_identifier` is the sole deviation from the issue's literal expected document, and it is load-bearing on an unstated assumption — that the DB schema tolerates additional properties on a definition. The issue says only that the field "is not consumed by this DB parser", which is a statement about consumption, not about `additionalProperties`. Nothing in the plan validates the assumption, and the rationale given for the retention contradicts the plan's own § Impact: § Consequences argues "removing it in a bug fix would change the published artifact for no verified gain", while § Impact states "this repository has no such consumer, and `grep` finds the file referenced only by the Dockerfile `COPY` and the tarball contract test". If the assumption is false, the fix ships and the database still crash-loops.
- Fix: Rewrite the § Consequences row-2 rationale and decision-log.md § [3] to name the real load-bearing assumption — that the DB SLC v2 schema does not set `additionalProperties: false` on a language definition — and state that the manual Personal check (Feasibility F1) is what validates it, so the tarball verified there MUST be the one that still carries `language_identifier`.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Dependencies bullet 1 — "Preinstalled on the `ubuntu-latest` and `ubuntu-24.04-arm` runners"
- Issue: the claim is unverified inside this repository. `.github/workflows/ci.yml` uses `jq` today only at the `Build IT test binary` step in the `build` job, which runs on `ubuntu-latest`. After task 3, the `build-slc` job's **aarch64** leg (`runs-on: ubuntu-24.04-arm`) hard-depends on `jq` through the delegated child, and a missing `jq` makes the child `die` with exit 2 and reds the leg. The ARM runner images carry a reduced tool set relative to the x86_64 images; I could not confirm from this repository that `jq` is among their preinstalled packages.
- Fix: In plan.md § Implementation Tasks, extend task 4 to also add a `jq` availability guarantee to the `build-slc` job — either an `Install jq` step (`sudo apt-get update && sudo apt-get install -y jq`) before `Run SLC tarball contract test`, or a cited confirmation in § Dependencies that `jq` ships preinstalled on `ubuntu-24.04-arm`.

## Requirement Quality

#### [COMPLETENESS_GAP] BLOCKER
- Location: plan.md § Implementation Tasks task 1 (final sentence) and § Verification § Scenario Coverage row 3; `container/slim-image/spec.md` § "Language definition contract is asserted by key path on both copies", clause 1
- Issue: no test in this plan demonstrates that any assertion discriminates. The spec's NEW clause 1 requires "a renamed root key or a re-typed field fails the check instead of passing unnoticed", but § Scenario Coverage row 3 maps that clause to the script itself, which is circular. The plan's only negative evidence is task 1's "Run it against the unchanged `build_info/language_definitions.json` and confirm it fails on the root key, on `arguments`, and on the absent `deprecation`" — and that run is confounded. The unchanged file has root key `languages`, so `.language_definitions[0]` evaluates to `null` and *every* per-definition assertion fails for one shared reason. `null | has("arguments")` does not even evaluate; `jq` errors with "null (null) has no keys" and exits 5. The implementer therefore sees three red lines that prove only the root-key assertion works, while an assertion that can never fail (Feasibility F2) looks equally verified. Clause 1 is also not testable as written — "asserted by its key path rather than by substring match" describes an implementation technique, not an observable outcome — and negative fixtures are what turn it into a pass/fail statement.
- Fix: Add a task between tasks 1 and 2 of plan.md § Implementation Tasks: create one-per-defect negative fixtures under `dist/tests/fixtures/language_definitions/` (each an otherwise-conforming document varying exactly one fact: root key renamed to `languages`; `deprecation` key removed; `arguments: ["lang=rust"]` re-added; `parameters` re-typed to `["lang=rust"]`; `udf_client_path.executable` changed), assert that `dist/tests/language_definitions_test.sh` exits non-zero on each one and exits zero on a conforming fixture, and reword the spec's NEW clause 1 to the observable form "each required field's absence, removal, or re-typing MUST fail the check". Then replace § Verification § Scenario Coverage row 3's test-name cell with those fixture cases.

## Task Breakdown

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Implementation Tasks task 1, versus `container/slim-image/spec.md` § "Language definitions file is present and well-formed" — "it MUST hold exactly one language definition"
- Issue: the spec's cardinality clause has no asserting check. Task 1 names the assertion `language_definitions_holds_one_conforming_definition` but enumerates only field-level checks against `[0]` — `aliases`, `protocol`, `parameters`, `udf_client_path.executable`, `deprecation`. A document with two definitions whose first conforms passes every assertion while violating the spec, and `deprecation`'s absence on a second definition would ship unseen.
- Fix: In plan.md § Implementation Tasks task 1, add an explicit array-length check to `language_definitions_holds_one_conforming_definition` — `jq -e '.language_definitions | length == 1'` — and add the two-definition case to the negative fixture set from the Requirement Quality fix.

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md § Implementation Tasks task 3; `dist/tests/slc_tarball_test.sh:521`
- Issue: task 3 deletes the very thing the retained cross-check reads, and does not say what replaces it. Line 521 derives the declared executable from `compact` (`declared="$(printf '%s' "$compact" | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p')"`), and task 3 orders "delete the `expected` substring array and the `compact` derivation" while also ordering "keep the cross-check that the declared `udf_client_path.executable` resolves to an executable file inside the extracted tree". An implementer following the letter either leaves a dangling reference or re-derives the value with `jq` — putting the key path `.language_definitions[0].udf_client_path.executable` into a second script and undercutting the plan's own "sole owner of the DB SLC v2 contract" claim in § Design § Architecture. The same shape recurs one level out: after this change `lang`/`rust` is encoded in the metadata's `parameters` and, in a different form, in `scripts/lib/script_languages.sh`'s `?lang=rust`, with nothing reconciling them — which is verbatim the drift argument § Consequences used to reject keeping `arguments`.
- Fix: In plan.md § Implementation Tasks task 1, give `dist/tests/language_definitions_test.sh` an optional second argument, the extracted tree root, and move the "declared executable resolves to an executable file in the tree" assertion into it, so the key path lives in exactly one script; then in task 3 reduce `slc_tarball_language_definitions_well_formed` to the `cmp` plus a single delegated call `bash "$HERE/language_definitions_test.sh" "$TREE/build_info/language_definitions.json" "$TREE"`. Add a note to plan.md § Design § Non-Goals stating that `lang=rust` remains encoded independently in `scripts/lib/script_languages.sh` and that no check reconciles the two.

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Design § Context, force 2 heading — "**The integration matrix cannot see this bug and never will.**"
- Issue: the claim contradicts the same document. plan.md § Dependencies commits to a follow-up that adds exactly the missing cover: "integration cover of the custom-SLC install path (extract the tarball into a container's `/exa/slc/<name>`, restart the database, assert a clean init)". "Never will" reads as a permanent property of the harness rather than a property of its present shape, and a later planner reading only § Context would conclude the follow-up is pointless.
- Fix: Change the force-2 heading in plan.md § Design § Context to "**The integration matrix does not exercise the path this bug lives on.**" and leave the body unchanged.

#### [PROSE_BLOAT] ADVISORY
- Location: plan.md § Impact, sentence 2
- Issue: 41 words carrying three separate claims in one sentence — "Nothing breaks on the BucketFS registration path that `scripts/install.sh` drives, because it reads the `SCRIPT_LANGUAGES` URL string and never this file, so existing `ALTER SYSTEM SET SCRIPT_LANGUAGES` values keep working unchanged." § Impact is governed, PR-facing prose under a 25-word sentence cap.
- Fix: Split it in plan.md § Impact into "The BucketFS registration path is unaffected: `scripts/install.sh` reads the `SCRIPT_LANGUAGES` URL string, never this file. Existing `ALTER SYSTEM SET SCRIPT_LANGUAGES` values keep working unchanged."
