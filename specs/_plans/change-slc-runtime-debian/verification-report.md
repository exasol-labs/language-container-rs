# Verification Report: change-slc-runtime-debian

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Alpine SLC runtime replaced with a curated `debian:trixie-slim` staged tree; all build/test/lint/format/license checks green, including the live-DB integration suite |
| Code review | 18 findings — standard: 15 fixed, expert: 3 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Licenses | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | Not measured with `cargo llvm-cov` in this run; every new module (`elf.rs`, `slc_surface.rs`) carries dedicated `_tests.rs` unit tests per this project's layout convention |
| Integration | Structural (tarball contract test, 18/18 assertions) + host build/lint/license gates + live-DB roundtrip suite, all green |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test --workspace --exclude it`) | 338 | 338 | 0 |
| `cargo-exasol-udf` crate (`cargo test -p cargo-exasol-udf`) | 41 | 41 | 0 |
| Live-DB integration (`db_roundtrip_all_scenarios`, dedicated `exasol/docker-db:2026.1.0`) | 1 (36 sub-scenarios) | 1 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `docker build --target artifact --output type=local,dest=<dir> .` completes; `lc-rs.tar.gz` produced | ✓ |
| `bash dist/tests/slc_tarball_test.sh <tarball>` — every assertion passes, size/floor printed | ✓ (18/18 assertions) |
| `tar -tvzf lc-rs.tar.gz` shows `/etc/hosts`, `/etc/resolv.conf`, usr-merge entries as symlinks | ✓ |
| `SLC_TARBALL=<tarball> cargo test -p it --features integration` — 0 failures | ✓ (dedicated `exasol/docker-db:2026.1.0` container, 36 sub-scenarios, 32.99s) |
| `bash dist/tests/os_licenses_test.sh` — passes, no apk/alpine/musl string | ✓ |
| `grep -c 'GCC Runtime Library Exception\|bzip2' dist/THIRD-PARTY-OS-LICENSES.md` — non-zero for both | ✓ |
| `bash dist/tests/about_toml_test.sh` + tarball carries both THIRD-PARTY files | ✓ |
| `cargo exasol-udf validate` on a built UDF `.so` — reports glibc/dependency summary, exit 0 | ✓ (via `cargo test -p cargo-exasol-udf` fixtures; equivalent coverage) |
| `cargo exasol-udf validate --deny-unknown-deps` on a staged-only `.so` — exit 0 | ✓ (`validate_allows_staged_dt_needed_under_flag`) |
| `cargo exasol-udf validate Cargo.toml` — exit non-zero, names it not a parseable ELF | ✓ (`validate_rejects_non_elf_input`) |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 52.89s
(0 warnings, 0 errors)
```

### Formatter

```
cargo fmt --check
(clean — one drift found and fixed mid-verification: three call sites in
crates/cargo-exasol-udf/tests/validate.rs left un-reformatted after the
review-fix batch that introduced the CdylibFixture/SharedStub parameter
structs; `cargo fmt` applied and re-verified clean)
```

### Licenses

```
cargo deny check licenses
licenses ok
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| container | slim-image | docker build produces the SLC artifact tarball | `dist/tests/slc_tarball_test.sh` | `slc_tarball_contains_executable_client` | Pass |
| container | slim-image | Builder toolchain and glibc runtime | `dist/tests/slc_tarball_test.sh` | `slc_client_dt_needed_is_expected_set` | Pass |
| container | slim-image | SLC builds natively for the host architecture | `dist/tests/slc_tarball_test.sh` | `slc_client_matches_host_arch_and_loader_resolves` | Pass |
| container | slim-image | Staged tree reproduces the donor's usr-merge layout | `dist/tests/slc_tarball_test.sh` | `slc_tarball_usr_merge_symlinks_match_arch` | Pass |
| container | slim-image | Runtime stage is slim and self-sufficient | `dist/tests/slc_tarball_test.sh` | `slc_tarball_has_no_shell_or_package_manager`, `slc_tarball_has_c_utf8_locale` | Pass |
| container | slim-image | Staged tree provides the documented UDF library surface | `dist/tests/slc_tarball_test.sh` | `slc_tarball_library_surface_present`, `slc_tarball_dt_needed_closure_is_complete`, `slc_tarball_nsswitch_modules_are_staged`, `slc_tarball_openssl_trust_path_resolves` | Pass |
| container | slim-image | Staged glibc defines the documented author floor | `dist/tests/slc_tarball_test.sh` | `slc_tarball_glibc_floor_matches_committed_value` | Pass |
| container | slim-image | Language definitions file is present and well-formed | `dist/tests/slc_tarball_test.sh` | `slc_tarball_language_definitions_well_formed` | Pass |
| container | slim-image | Staged tree passes an in-build chroot self-test | `Dockerfile` staging stage (build-time) | in-build `chroot` self-test | Pass |
| container | slim-image | Staged tree passes an in-build chroot self-test (structural proof) | `dist/tests/slc_tarball_test.sh` | `slc_client_matches_host_arch_and_loader_resolves` | Pass |
| container | slim-image | Debian-staged SLC passes the db-roundtrip integration suite | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` | Pass |
| container | slim-image | Staged tarball carries only the curated runtime surface | `dist/tests/slc_tarball_test.sh` | `slc_tarball_has_no_shell_or_package_manager`, `slc_tarball_staged_surface_within_ceiling` | Pass |
| container | slim-image | SLC tarball ships the /conf resolver symlinks | `dist/tests/slc_tarball_test.sh` | `slc_tarball_conf_resolver_symlinks` | Pass |
| container | slim-image | Runtime image bundles the IANA zoneinfo database | `dist/tests/slc_tarball_test.sh` | `slc_tarball_zoneinfo_is_regular_file` | Pass |
| container | os-license-notices | OS-layer license generator boilerplate is committed under dist/ | `dist/tests/os_licenses_test.sh` | `os_boilerplate_committed_and_license_sets_agree` | Pass |
| container | os-license-notices | The generator renders a complete OS-license manifest via cargo-about | `dist/tests/os_licenses_test.sh` | `os_manifest_covers_staged_library_set` | Pass |
| container | os-license-notices | The Dockerfile ships the generated OS-license manifest into the tarball | `dist/tests/slc_tarball_test.sh` | `slc_tarball_carries_notice_bundles` | Pass |
| container | os-license-notices | Distributed tarball carries the OS-layer notice at /exaudf | `dist/tests/slc_tarball_test.sh` | `slc_tarball_carries_notice_bundles`, `slc_tarball_os_notice_has_no_apk_references` | Pass |
| container | crate-license-notices | Target set reflects the shipped glibc binary | `dist/tests/about_toml_test.sh` | `about_toml_lists_gnu_triples` | Pass |
| container | crate-license-notices | Generated manifest ships in the tarball for each architecture | `dist/tests/slc_tarball_test.sh` | `slc_tarball_carries_notice_bundles` | Pass |
| tools | cargo-exaudf | validate accepts a compatible .so | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_accepts_named_entries_and_reports_platform_summary` | Pass |
| tools | cargo-exaudf | validate rejects a .so missing any entry symbol | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_a_so_without_any_named_entry_symbol`, `validate_rejects_non_elf_input` | Pass |
| tools | cargo-exaudf | validate reports the artifact's glibc version floor | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_reports_glibc_floor_summary` | Pass |
| tools | cargo-exaudf | validate reports the artifact's glibc version floor (unit) | `crates/cargo-exasol-udf/src/elf_tests.rs` | `max_glibc_version_picks_highest_reference`, `max_glibc_version_is_none_without_verneed` | Pass |
| tools | cargo-exaudf | validate rejects an artifact above the SLC glibc floor | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_glibc_above_floor` | Pass |
| tools | cargo-exaudf | validate rejects an artifact above the SLC glibc floor (unit) | `crates/cargo-exasol-udf/src/slc_surface_tests.rs` | `floor_check_rejects_newer_glibc`, `committed_floor_is_the_published_container_floor` | Pass |
| tools | cargo-exaudf | validate warns on dynamic dependencies outside the SLC library surface | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_warns_on_unknown_dt_needed` | Pass |
| tools | cargo-exaudf | validate warns on dynamic dependencies outside the SLC library surface (unit) | `crates/cargo-exasol-udf/src/slc_surface_tests.rs` | `unknown_sonames_exclude_loader_and_vdso` | Pass |
| tools | cargo-exaudf | validate escalates unknown dynamic dependencies on request | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_denies_unknown_dt_needed_with_flag`, `validate_allows_staged_dt_needed_under_flag` | Pass |

## Notes

**Mid-implementation correction (bzip2).** The plan originally assumed `exaudfclient` links bzip2 dynamically and required pinning/asserting that (`libbz2-dev` in the builder, a `slc_client_links_bzip2_dynamically` test). Investigation during implementation proved this false: `exaudfclient` links no bzip2 at all, dynamic or static, on any build. Root cause: `exarrow-rs`'s only use of the `bzip2` crate is behind its CSV `IMPORT`/`EXPORT` local-file-compression feature, a code path this project's `ExaConnection` usage (`query`/`query_for_each`/`execute`) never reaches, so Rust's dead-code elimination drops the whole unreachable unit. Corrected per explicit operator direction: `libbz2-dev` removed from the builder, the bzip2-link CI assertion dropped entirely (replaced with an explicit "neither `libbz2` nor `libzmq` is `DT_NEEDED`" assertion), `libbz2.so.1` staging for UDF authors' own crates kept unchanged, and bzip2 attribution kept in the license bundle. Full writeup in `decision-log.md` entry [5].

**Two hotfixes found during implementation, both fixed and verified:**
- A build-time debug trace (`/slc/tmp/exaudf_started.txt`, written by the in-build `chroot` self-test) was leaking into every shipped tarball. Fixed (`find /slc/tmp -mindepth 1 -delete`) and guarded by a new test assertion (`slc_tarball_tmp_is_empty_and_world_writable`), which also caught that `tar`'s default extraction drops archived file modes for non-root — the test now extracts with `tar -xzpf` to test the same guarantee BucketFS extraction (as root) provides.
- The SLC library surface (15 sonames) was independently declared in three places (Dockerfile staging loop, `validate`'s allowlist, the tarball test) with nothing enforcing agreement — extracted to one committed `crates/cargo-exasol-udf/slc-library-surface.txt`, read by all three, mirroring the existing glibc-floor pattern.

**Live-DB integration suite: run and green, via a dedicated container.** A separate `exasol/docker-db` container was already running in this environment for an unrelated peer session (`lakehouse-engine-rs`); rather than reuse or disrupt it, a second dedicated `exasol/docker-db:2026.1.0` container was started on non-conflicting host ports (18563/12581) specifically for this verification. `db_roundtrip_all_scenarios` (36 sub-scenarios: scalar/SET/EMITS, connect-back query/insert/crunch/stream/cluster-ip, resolver, timestamp precision/timezone, error/edge cases) passed in 32.99s against the version-bumped (`0.23.0`) SLC tarball and freshly rebuilt `test-udfs/*` fixtures.

**Pre-existing bug found in `scripts/ci-it-local.sh`, unrelated to this plan, not fixed here (out of scope).** The script's default `DB_SERIES=db-2026-1` is passed as a Cargo feature (`--features integration,db-2026-1`), but the `it` crate's `Cargo.toml` defines only an `integration` feature — DB version selection is actually controlled purely at runtime via the `EXASOL_VERSION`/`EXASOL_DB_SERIES` env vars (`crates/it/src/lib.rs`), not compile-time features. This makes every invocation of `scripts/ci-it-local.sh` fail immediately at its "Build IT test binary" step with `error: the package 'it' does not contain this feature: db-2026-1`, before ever starting a DB — independently confirmed against a clean run. This plan's task 3.2 only touched the script's `docker build` invocation line, per its stated scope; the `DB_SERIES` feature-flag bug predates this plan and CI's own `integration` job (`.github/workflows/ci.yml`) does not hit it (it does not go through this script). **Recommend a follow-up fix** (drop the bogus feature from the `cargo test --no-run` invocation, or add real per-version Cargo features to the `it` crate if that granularity is wanted) — filed as a note here rather than a GitHub issue since it's a one-line script defect, not a feature gap.

**aarch64 coverage** is structurally-only in this session (this environment is x86_64) — matches the plan's own "Accepted verification gaps" section (CI's `ubuntu-24.04-arm` runner plus a manual Exasol Personal run cover that leg; `exasol/docker-db` is amd64-only).

**Code review:** 18 findings (15 standard, 3 expert), all fixed and re-verified. Notable: `validate::run` was decomposed into single-responsibility helpers; a silently-discarded `--deny-unknown-deps` typo now errors; `goblin` narrowed to non-default features; a duplicate ELF read in `build.rs` eliminated; several test-quality issues (a host-fragile helper, a hand-rolled `TempDir` beside an existing `tempfile` dependency, 4-argument fixture helpers) cleaned up.
