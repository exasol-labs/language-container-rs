# Verification Report: fix-slc-metadata-schema

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All automated and buildable checks are green. One manual gate (Personal custom-SLC install) could not be executed in this environment; it requires a live Exasol Personal deployment, is the plan's own designated non-CI verification step, and MUST pass before merge per plan.md § Dependencies. |
| Code review | 5 findings — 5 fixed (1 standard, 4 expert) |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ (4/5 — see Notes for the one deferred to a live Personal deployment) |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | Not measured (no `cargo llvm-cov` run this pass) |
| Integration | 26/26 `it` scenarios pass (full db-roundtrip suite, unaffected-by-change confirmed empirically) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test`) | 331 | 331 | 0 |
| Integration (`cargo test -p it --features integration`, local replay against `exasol/docker-db:2026.1.0`) | 1 (`db_roundtrip_all_scenarios`, 26 sub-scenarios) | 1 | 0 |
| Shell contract — `language_definitions_test.sh` | 5 | 5 | 0 |
| Shell contract — `language_definitions_fixtures_test.sh` | 13 | 13 | 0 |
| Shell contract — `about_toml_test.sh` (regression) | 2 | 2 | 0 |
| Shell contract — `install-personal-test.sh` (regression) | 22 assertions | 22 | 0 |
| Shell contract — `slc_tarball_test.sh` (full, against Docker-built artifact) | 20 | 20 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `bash dist/tests/language_definitions_test.sh build_info/language_definitions.json` | ✓ 5 `PASS:` lines, exit 0 |
| `bash dist/tests/language_definitions_fixtures_test.sh` | ✓ 13 `PASS:` lines, exit 0 |
| Docker build + `slc_tarball_test.sh` on the built tarball | ✓ 20 `PASS:` lines, exit 0 |
| `tar -xzOf lc-rs.tar.gz ./build_info/language_definitions.json \| jq .` | ✓ root key `language_definitions`, `parameters: [{"key":"lang","value":"rust"}]`, `deprecation: null`, no `arguments` key |
| Install as custom SLC on Exasol Personal, read DB init log | Not executed — no live Exasol Personal deployment available in this environment. This is the plan's own designated manual gate (§ Dependencies: "MUST pass before merge, not after"); a human must run it before the PR is merged. |

## Tool Evidence

### Linter

```
cargo clippy --workspace --all-targets --all-features -- -D warnings
exit 0, no warnings
```

### Formatter

```
cargo fmt --all -- --check
exit 0, no diff
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| container | slim-image | Language definitions file is present and well-formed | `dist/tests/language_definitions_test.sh` | `language_definitions_document_is_valid_json`, `language_definitions_declares_schema_version_2`, `language_definitions_root_key_is_language_definitions`, `language_definitions_holds_one_conforming_definition`, `language_definitions_definition_has_no_arguments_key` | Pass |
| container | slim-image | Language definitions file is present and well-formed | `dist/tests/slc_tarball_test.sh` | `slc_tarball_language_definitions_well_formed` | Pass |
| container | slim-image | Language definition contract is asserted by key path on both copies | `dist/tests/language_definitions_fixtures_test.sh` | 12 rejection cases + `language_definitions_accepts_conforming` | Pass |

## Notes

- **Code review found and closed a real coverage gap.** The as-planned fixture set (6 fixtures) left 6 of the contract script's 11 assertion branches unproven, one of them unreachable by any of those fixtures. The expert fix pass added 6 more fixtures (`schema_version_wrong`, `definitions_not_array`, `two_definitions`, `aliases_changed`, `protocol_changed`, `deprecation_non_null`) plus a `malformed.json` case, bringing the driver to 13 cases covering all branches. plan.md's Scenario Coverage and Manual Testing tables were updated to match (four → five `PASS:` lines; six → thirteen).
- **The driver's rejection assertion was hardened.** The original `assert_rejected` credited any non-zero exit as proof, which would have silently passed all five negatives if `jq` went missing, and could not tell an intended failure from a collateral one (verified live on `root_key_renamed.json`, which fails three assertions at once). It now checks for the specific `FAIL:` message expected from each fixture.
- **Integration suite confirmed unaffected, empirically, not just by design argument.** The `it` crate registers SLCs via `ALTER SESSION SET SCRIPT_LANGUAGES`, never reading `build_info/language_definitions.json` — this plan's own rationale for why the metadata bug shipped past a green CI matrix. Running the full local `db-roundtrip` suite (26 scenarios) against the corrected metadata confirms no regression.
- **Environment gap, not a defect:** Personal custom-SLC install verification needs a live Exasol Personal deployment (AWS-hosted, per `exasol:setup-personal`), which this session did not have provisioned. Every other acceptance criterion in issue #90 is closed with direct evidence above.
