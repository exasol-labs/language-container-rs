# Verification Report: fix-personal-slc-install

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Both defects from issue #110 are fixed and verified live. A Rust scalar UDF now returns `ok` on the macOS arm64 exasol-personal v2.3.0-rc3 deployment via both `scripts/install.sh --deployment` and the launcher's own `exasol slc custom install`/`update`, instead of `VM error: Internal error: VM crashed (SQL state: 22002)`. |
| Code review | 6 findings — 6 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ (see Notes: 16 pre-existing, macOS-only failures outside this plan's scope) |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed | Ignored |
|------|-----|--------|--------|---------|
| Unit (`cargo test`, default-members) | 370 | 354 | 16 (pre-existing, unrelated — see Notes) | 2 |
| Shell (`scripts/tests/install-personal-test.sh`) | 100 assertions | 100 | 0 | 0 |
| Container contract (`dist/tests/slc_tarball_test.sh`) | 20 | 20 | 0 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| Build the SLC artifact for arm64 (`docker build --target artifact --output type=local,dest=/tmp/lc-out .`) | ✓ — succeeded, chroot self-test passed |
| `tar -tzf lc-rs.tar.gz \| grep -E '^\./(proc\|buckets\|var/tmp)/$'` | ✓ — all three present |
| `dist/tests/slc_tarball_test.sh` against the built artifact (run inside a Linux arm64 container, since the script needs Linux-only tooling) | ✓ — 20/20 passed, including the new `slc_tarball_ships_sandbox_skeleton` |
| `scripts/install.sh --deployment default` against the live deployment (no `.connection.sshPort` published — forces the shared-directory mechanism) | ✓ — placed the tree at `local/runtime/vm-shared/exa/bucketfs/bfsdefault/default/rustslc/`, preserved `R`/`JAVA`/`PYTHON3`, registered `RUST=...rustslc...` |
| `exaudf/exaudfclient` present and executable on host after install | ✓ — mode 755 |
| Rust scalar UDF (`health_check`, matching the issue's exact repro) called after `scripts/install.sh` install | ✓ — returned `ok` |
| `exasol slc custom update --alias RUST --source lc-rs.tar.gz --language rust` (the launcher's own install path, issue's originally-reported path) | ✓ — restarted the deployment, `SCRIPT_LANGUAGES` carried the new `RUST=...__builtin__/slc/custom-rust...` entry |
| Same `health_check` UDF called after the `exasol slc` install | ✓ — returned `ok` |
| `scripts/install.sh --deployment default --bucket nosuchbucket` | ✓ — failed naming the unserved service/bucket pair, removed nothing |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
0 errors, 0 warnings
```

### Formatter

```
cargo fmt --check
no diff
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| container | slim-image | SLC tarball ships the sandbox mount-point skeleton | `dist/tests/slc_tarball_test.sh` | `slc_tarball_ships_sandbox_skeleton` | Pass |
| container | personal-install | Connection details are read fresh on every run | `scripts/tests/install-personal-test.sh` | `reads_ssh_port_from_deployment_json` | Pass |
| container | personal-install | SLC is deployed via filesystem BucketFS reconciliation | manual, live deployment | — | Pass |
| container | personal-install | Registration targets the exaudfclient executable | `scripts/tests/install-personal-test.sh` | `fragment_points_at_executable_no_leading_slash`, `local_mechanisms_share_the_registration_inputs` | Pass |
| container | personal-install | Registration is system-scoped and preserves existing entries | `scripts/tests/install-personal-test.sh` | `preserves_existing_script_languages` | Pass |
| container | personal-install | A registered Rust UDF executes on Personal | manual, live deployment | — | Pass |
| container | personal-install | Deployment backend selects the transport | `scripts/tests/install-personal-test.sh` | `selects_transport_from_backend` | Pass |
| container | personal-install-local | Local install resolves the DB password from the deployment directory | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor` | Pass |
| container | personal-install-local | Local connection details resolve from the deployment directory | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor` | Pass |
| container | personal-install-local | Command-line flags override descriptor-derived local values | `scripts/tests/install-personal-test.sh` | `cli_port_overrides_local_descriptor`, `cli_host_overrides_local_descriptor` | Pass |
| container | personal-install-local | A local descriptor that omits the SQL port is reported | `scripts/tests/install-personal-test.sh` | `resolves_local_defaults_when_db_port_absent` | Pass |
| container | personal-install-local | The deployment directory selects the local install mechanism | `scripts/tests/install-personal-test.sh` | `selects_local_mechanism_from_deployment_directory` | Pass |
| container | personal-install-local | The shared-directory mechanism extracts into the deployment's own BucketFS directory | `scripts/tests/install-personal-test.sh` | `extracts_slc_into_shared_bucketfs` | Pass |
| container | personal-install-local | The shared-directory destination is checked before anything is removed | `scripts/tests/install-personal-test.sh` | `rejects_path_unsafe_name_components`, `resolves_shared_bucketfs_dir_from_mapping`, `extracts_slc_into_shared_bucketfs` | Pass |
| container | personal-install-local | Both local mechanisms register through the install script | `scripts/tests/install-personal-test.sh` | `local_mechanisms_share_the_registration_inputs` | Pass |
| container | personal-install-local | Re-running the local install replaces the installed SLC | `scripts/tests/install-personal-test.sh` | `extracts_slc_into_shared_bucketfs`, `preserves_existing_script_languages` | Pass |
| container | personal-install | Registration refuses a SCRIPT_LANGUAGES value it could not read (review finding 4.8) | `scripts/tests/install-personal-test.sh` | `refuses_to_register_when_the_current_value_cannot_be_read` | Pass |

## Notes

**16 pre-existing, unrelated test failures**, all in `crates/cargo-exasol-udf` (`elf::tests::read_returns_*`, `tests/build.rs`, `tests/validate.rs`). Root cause: these tests invoke `rustc --crate-type=cdylib` and expect Linux ELF/`.so` semantics (lazy binding for an intentionally-undefined symbol, `readelf`-driven inspection). On macOS, `rustc` emits a Mach-O `.dylib` and the linker rejects the same undefined symbol at link time instead of leaving it unresolved for dynamic lookup. Verified byte-for-byte identical failure sets on unmodified `origin/main` in a disposable git worktree (both the 3 `elf::` failures and the 6 `build.rs`/`validate.rs` failures). None of this plan's changes touch `crates/cargo-exasol-udf`. This is a macOS-host limitation that needs a Linux runner; not a regression.

**`dist/tests/slc_tarball_test.sh` needs Linux-only tooling** (`readelf`, GNU `bash` with `declare -A`, `du -sb`, `jq`) absent on this macOS host. Verified instead inside a `linux/arm64` Alpine container with the tools installed, against the actual built artifact: all 20 assertions pass.

**Manual verification went beyond the plan's own checklist**: the plan's `## Verification > Manual Testing` table assumed an x86_64 Linux host would run the checks; this session ran them on the actual target environment the user asked for — a live macOS arm64 exasol-personal v2.3.0-rc3 deployment (Colima-backed Docker) — via both local install mechanisms, and confirmed a real UDF call succeeds end to end on both.

**Follow-up issues to file** (out of this plan's scope, surfaced during implementation and review):
1. A `feature`-labelled issue to delete the SSH mechanism, `deployment_ssh_port`, `deployment_key_path`, `extract_slc_into_bucketfs`, `--ssh-user`, and their tests once no supported local Personal deployment publishes SSH inputs (planned in `plan.md`'s Dead Code Removal section).
2. A `bug`-labelled issue: on the cloud Personal branch (`scripts/install.sh`, around lines 798-802), passing `--scope system` skips reading the current `SCRIPT_LANGUAGES` value before overwriting it, so it can drop every pre-existing language the same way finding 4.8 closed on the Personal-local path. Not touched by this plan (different branch, not named in the review findings).