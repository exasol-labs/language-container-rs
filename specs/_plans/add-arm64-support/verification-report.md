# Verification Report: add-arm64-support

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All automated checks green; the x86_64 integration suite passed live (29 scenarios). aarch64 end-to-end stays manual per decision [6] (no arm64 Exasol DB image). |
| Code review | 3 findings — 3 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests (unit) | ✓ 306 passed |
| Integration (x86_64, live DB) | ✓ 29 scenarios |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ (unit + x86_64 IT; aarch64 manual out of phase) |
| Manual Tests (aarch64/Personal) | Deferred — manual/live per decision [6]; no arm64 DB image |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Rust unit + doc (`cargo test`) | 311 | 306 | 5 |
| Shell — `dist/tests/about_toml_test.sh` | 2 | 2 | 0 |
| Shell — `scripts/tests/install-personal-test.sh` | 18 | 18 | 0 |
| Integration — `it` db-roundtrip (`ci-it-local.sh`, x86_64 live DB, `exasol/docker-db:2026.1.0`) | 1 (29 scenarios) | 1 | 0 |
| Rust cargo-exaudf integration (`tests/build.rs`) | — | — | 3 (`#[ignore]`d; need a musl toolchain) |

The 5 ignored Rust tests are the 3 `cargo-exaudf` build-integration tests (`build_produces_musl_so`, `build_installs_missing_target`, `build_honors_target_override`) plus 2 pre-existing `cli.rs` ignored tests. They build a real musl `.so`; run with `cargo test -p cargo-exasol-udf -- --ignored` on a musl-capable host.

### Manual Tests

| Test | Result |
|------|--------|
| aarch64 `docker build` → ELF ARM64 with correct PT_INTERP | Deferred (no arm64 CI host; manual field test) |
| Dockerfile fail-fast on empty triplet/loader | Validated at shell level by the P0 expert agent across 5 scenarios; live `docker build` deferred |
| `cargo exasol-udf build` on aarch64 host | Deferred (manual, aarch64 host) |
| `scripts/install-personal.sh` against live Personal + restart idempotency | Deferred (no arm64 DB image; manual/live) |

## Tool Evidence

### Build / Tests

```
cargo build --release            → exit 0
cargo test                       → exit 0 (306 passed, 0 failed, 5 ignored)
```

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings   → exit 0 (0 warnings)
```

### Formatter

```
cargo fmt --check                → exit 0 (no changes)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| tools | cargo-exaudf | build produces a fully-static musl .so | `crates/cargo-exasol-udf/src/build_tests.rs` | `host_triple_maps_arch` | Pass |
| tools | cargo-exaudf | build produces a fully-static musl .so (real build) | `crates/cargo-exasol-udf/tests/build.rs` | `build_produces_musl_so` | Ignored (musl host) |
| tools | cargo-exaudf | installs the musl target when missing | `crates/cargo-exasol-udf/tests/build.rs` | `build_installs_missing_target` | Ignored (musl host) |
| tools | cargo-exaudf | honors an explicit target override | `crates/cargo-exasol-udf/src/build_tests.rs` (parse) + `tests/build.rs` (e2e) | `parse_build_args_*` (Pass); `build_honors_target_override` (Ignored) | Pass / Ignored |
| container | slim-image | builder toolchain + glibc runtime | `crates/it/tests/` | `db_roundtrip_alpine` | Gated — x86_64 IT suite |
| container | slim-image | builds natively for host architecture | IT (x86_64) + manual (aarch64) | `db_roundtrip_alpine` / field build | Gated / Deferred |
| container | slim-image | empty triplet/loader fails with a named error | `Dockerfile.alpine` guard (task 0.3) | code review + manual | Pass (review + shell-harness proof) |
| container | crate-license-notices | covers every shipped architecture | `dist/tests/about_toml_test.sh` | `about_toml_lists_gnu_triples` | Pass |
| container | crate-license-notices | target set reflects the shipped glibc binary | `dist/tests/about_toml_test.sh` | `about_toml_lists_gnu_triples`, `about_toml_comments_glibc_rationale` | Pass |
| container | crate-license-notices | manifest ships in the tarball | `crates/it/tests/` | `tarball_carries_third_party_licenses` | Gated — x86_64 IT suite |
| container | personal-install | connection details read fresh every run | `scripts/tests/install-personal-test.sh` | `reads_ssh_port_from_deployment_json` | Pass |
| container | personal-install | deployed via filesystem BucketFS reconciliation | live Personal | manual | Deferred |
| container | personal-install | registration targets the exaudfclient executable | `scripts/tests/install-personal-test.sh` | `fragment_points_at_executable_no_leading_slash` | Pass |
| container | personal-install | system-scoped, preserves existing entries | `scripts/tests/install-personal-test.sh` | `preserves_existing_script_languages` | Pass |
| container | personal-install | a registered Rust UDF executes on Personal | live Personal | manual | Deferred |

## Notes

- **All 7 phases (P0–P7) landed as one PR bundle.** The plan's per-phase version-bump tasks are consolidated into a single `0.21.3 → 0.22.0` (`feat`, minor) bump applied by the orchestrator after this report; `Cargo.toml`/`Cargo.lock` were untouched during implementation.
- **P0 x86_64 no-op proven, not assumed.** The expert agent diffed the derived-triplet build against the hardcoded one and confirmed identical staged trees. The fail-fast guards (empty `TRIPLET`/`LOADER`) sit before `mkdir` so a bad derive aborts with a named error instead of silently mis-staging the loader — the `22002 VM crashed` failure mode from decision [8].
- **Within-PR churn (intentional):** P0 adds `targets/aarch64-unknown-linux-musl-dylib.json`; P7 removes both target JSONs and the `COPY targets/` line. Net: both JSONs absent, per decision [4].
- **Cross-language triplication resolved as a cross-reference.** `crates/it/src/lib.rs` `SlcRef::script_languages` rebuilds the same `SCRIPT_LANGUAGES` fragment in Rust; full dedup is infeasible across the shell/`.so` boundary, so review fix added a sync-cross-reference comment in `scripts/lib/script_languages.sh` (finding `[INFORMATION_LEAKAGE]`).
- **Review fixes:** `[MISSING_BOUNDARY_TEST]` — `parse_build_args` now returns `Result` and errors on a dangling `--target`, with 5 fast unit tests. `[SKIPPED_TEST]` — `dist/tests/about_toml_test.sh` wired into the x86_64 unit-test CI leg. `[INFORMATION_LEAKAGE]` — cross-reference comment as above.
- **CI:** the arm64 leg (`ubuntu-24.04-arm`) runs `cargo build --workspace` + unit tests only; IT stays x86_64 (decision [6]). Both new shell test suites run in the x86_64 unit-test job.
- **Integration suite passed live.** `scripts/ci-it-local.sh` (replays the CI `integration` job: fresh `0.22.0` SLC build → `exasol/docker-db:2026.1.0` → 29 db-roundtrip scenarios) → exit 0, all scenarios ok. Ran with the fix-validating memory config (`DB_MEM='4 GiB' MEM=12g SHM=2g`).
- **Local-harness note (not a code defect, follow-up):** the first IT run false-failed with a fingerprint mismatch (`.so` at `0.21.3` vs SLC `0.22.0`) because `scripts/ci-it-local.sh`'s step-2 `-p` build list omits three fixtures (`numeric-temporal-emit`, `numeric-temporal-ingest`, `handshake-meta`) that `.github/workflows/ci.yml` DOES build. CI is unaffected (clean runner builds all fixtures fresh); only stale local `target/release` copies triggered it. Rebuilding those three at `0.22.0` made the suite green. Worth syncing `ci-it-local.sh`'s list to `ci.yml` in a separate change.
- **Deferred to manual field test:** all aarch64/Personal live scenarios (no arm64 Exasol DB image exists).
