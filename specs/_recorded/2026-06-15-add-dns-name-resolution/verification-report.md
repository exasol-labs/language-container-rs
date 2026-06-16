# Verification Report: add-dns-name-resolution

## VERDICT: PASS

All automated checks pass, and the full integration suite passed against a live Exasol 2025.1.11 container — all 13 scenarios green, including both new DNS scenarios. The end-to-end run confirms the core deliverable: the `packager`-stage resolver symlinks (`/etc/hosts → /conf/hosts`, `/etc/resolv.conf → /conf/resolv.conf`) make DNS resolve inside the UDF sandbox.

---

## Automated Checks

| Step | Command | Result |
|------|---------|--------|
| Build | `cargo build --release` | ✅ Exit 0 |
| Unit tests | `cargo test` | ✅ 0 failures (2 pre-existing failures in `exa-udf-runtime/tests/dispatch.rs` confirmed pre-existing on `main` before this PR) |
| Lint | `cargo clippy --exclude it --workspace -- -D warnings` | ✅ 0 warnings |
| Format | `cargo fmt --check` | ✅ No changes |
| IT crate compile | `cargo build -p it --features integration,db-2026-1` | ✅ Exit 0 |

### Pre-existing test failures (not introduced by this PR)

Confirmed by running the same tests on `main` before `git stash pop`:

| Test | Error |
|------|-------|
| `scalar_dispatch_full_protocol` | `left: 4, right: 9` (dispatch.rs:97) |
| `annotated_schema_mismatch_closes_session` | ABI version mismatch: expected 3, found 4 (dispatch.rs:207) |

These existed before this branch; out of scope for this plan.

---

## Scenario Coverage

| Scenario | Test Location | Status |
|----------|---------------|--------|
| `resolv_udf` unit: localhost resolves to parseable IP | `test-udfs/resolv-udf/src/lib.rs::resolves_localhost_to_ip` | ✅ Passes |
| `resolv_udf` unit: unresolvable host yields `UdfError::User` | `test-udfs/resolv-udf/src/lib.rs::errors_on_unresolvable_host` | ✅ Passes |
| `resolv_udf` unit: non-string input yields `UdfError::Type` | `test-udfs/resolv-udf/src/lib.rs::errors_on_non_string_input` | ✅ Passes |
| SLC tarball ships `/conf` resolver symlinks | IT: `db_roundtrip_all_scenarios` → uploads tarball from `SLC_TARBALL` | ✅ Passed (Exasol 2025.1.11) |
| `resolv_udf` resolves external hostname to valid `IpAddr` | IT: `resolv_udf_resolves_external_host` | ✅ Passed (Exasol 2025.1.11) |
| `resolv_udf` errors on unresolvable hostname (F-UDF-CL-RUST- prefix) | IT: `resolv_udf_errors_on_unresolvable_host` | ✅ Passed (Exasol 2025.1.11) |

---

## Code Review Findings & Resolutions

| Finding | Severity | Resolution |
|---------|----------|------------|
| `resolv-udf` missing from `ci.yml` build job | Blocking | Fixed: added `-p resolv-udf` |
| `resolv-udf` missing from `ci-it-local.sh` build list | Blocking | Fixed: added `-p resolv-udf` |
| `install.sh` header described deleted `docker export` flow | Minor | Fixed: updated comment |
| `ci-it-local.sh` `SKIP_SLC_BUILD` doc said "image" not "tarball" | Minor | Fixed: updated comment |
| `ci-it-local.sh` mid-script `cleanup()` deleted the SLC tarball before tests ran (found by live run, not review) | Blocking | Fixed: split into `cleanup()` (container only) + `on_exit()` (also removes `SLC_DIR`) |

---

## Dead Code Removed

| Symbol | Location | Removed By |
|--------|----------|------------|
| `fn export_image_filesystem` | `crates/it/src/lib.rs` | Task 3.2 |
| `const SLC_IMAGE` | `crates/it/src/lib.rs` | Task 3.2 |
| `docker create`/`export\|gzip` block | `scripts/install.sh` | Task 4.1 |
| `docker build -t slc-rs-slim:dev` | `scripts/ci-it-local.sh` | Task 5.1 |
| `docker save slc-rs-slim:dev` + `slc-image` artifact | `.github/workflows/ci.yml` build-slc | Task 6.1 |
| `docker load -i slc-image.tar` | `.github/workflows/ci.yml` integration | Task 6.2 |
| `docker load` + `docker export\|gzip` in release | `.github/workflows/ci.yml` release | Task 6.3 |

---

## Files Changed

| File | Change |
|------|--------|
| `Dockerfile.alpine` | Added `packager` + `artifact` stages |
| `test-udfs/resolv-udf/Cargo.toml` | New crate |
| `test-udfs/resolv-udf/src/lib.rs` | New implementation + unit tests |
| `Cargo.toml` | Added `resolv-udf` to workspace; bumped version to 0.9.0 |
| `crates/it/src/lib.rs` | Rewrote `load_slc()`, removed `export_image_filesystem` + `SLC_IMAGE` |
| `crates/it/tests/db_roundtrip.rs` | Added `resolv_udf` upload + two DNS scenarios |
| `scripts/install.sh` | Replaced docker export with `--target artifact --output` |
| `scripts/ci-it-local.sh` | Same replacement; exports `SLC_TARBALL` |
| `.github/workflows/ci.yml` | `build-slc`/`integration`/`release` use `slc-tarball` artifact |
