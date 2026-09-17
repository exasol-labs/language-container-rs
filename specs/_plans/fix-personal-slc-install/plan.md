# Plan: fix-personal-slc-install

## Summary

Ship the sandbox mount-point directory skeleton inside the SLC tarball so Rust UDFs run on an Exasol Personal deployment that mounts the container read-only. Teach `scripts/install.sh --deployment` a second way to place the container on a local deployment: extract it into the host directory the deployment shares into its VM. Keep the SSH transport for a deployment that still publishes SSH inputs.

## Design

### Context

GitHub issue #110 reports two independent defects behind one symptom on Exasol Personal.

Defect A stops every Rust UDF. The SLC tarball is staged `FROM scratch` and carries only the directories the curated library surface needs. Personal mounts both official and custom containers into the database container read-only, with no writable layer. Its sandbox setup creates its mount points with `mkdir`. That call succeeds as a no-op against a full-distribution container such as the official Python3 SLC. It fails against the Rust SLC. The first absent path reports `cannot create directories: Read-only file system`, and every UDF call returns `22002 VM crashed` before any Rust code runs. The issue's follow-up comment verified the fix: pre-creating the skeleton in the tarball makes a scalar Rust UDF return its expected result on the same deployment.

Defect B stops `scripts/install.sh --deployment` on a current local Personal deployment. That path reads `.connection.sshPort` from `deployment.json` and a key at `local/node_access.pem`. Current local deployments publish neither. A live local deployment on this workstation carries a `.connection` object of `host`, `displayHost`, `publicIp`, `dbPort`, `username`, `insecureSkipCertValidation` and `shellSupported`, and no `local/node_access.pem`. The install therefore fails before it transfers anything.

Such a deployment shares a host directory into its VM instead. Its own BucketFS mapping file under `local/runtime/vm-shared/exa/` states which VM-side path serves each service and bucket pair. A live check on this workstation extracted an SLC tarball into the host directory that mapping names for `bfsdefault`/`default`. A Python UDF on the running deployment then listed the tree at `/buckets/bfsdefault/default/<name>/`. It reported `exaudf/exaudfclient` as mode `755`, readable and executable. The engine reconciled the bucket with no restart, no SSH session and no other tool. That is the same reconciliation the SSH mechanism already relies on, reached by writing the same bytes to the same place from the host side.

- **Goals**: Make a Rust UDF run on a read-only-mounted Personal deployment. Make `--deployment` work on a local deployment that publishes no SSH inputs, and keep it working on one that does.
- **Non-Goals**: Delegate any part of the install to the `exasol` launcher CLI. Change the BucketFS HTTP transport. Change the cloud Personal path. Change what a UDF author builds or uploads. Fix the launcher or the engine.

### Decision

#### Architecture

Defect A adds one committed data file and reads it from two places that already exist.

```
dist/slc-sandbox-skeleton.txt   (the one list of mount-point names)
        │
        ├──▶ Dockerfile staging stage  ──▶ lc-rs.tar.gz carries the directories
        │
        └──▶ dist/tests/slc_tarball_test.sh ──▶ asserts the same names
```

This mirrors `crates/cargo-exasol-udf/slc-library-surface.txt`, which already feeds both the staging loop and the contract test. A second hand-maintained copy of the list is how the shipped skeleton and the asserted skeleton drift apart.

Defect B varies one step of the local branch and leaves the rest of it single-sourced.

```
--deployment <name>
        │
        ▼
 deployment_backend()  ── not "local" ──▶ BucketFS HTTP transport (unchanged)
        │ "local"
        ▼
 resolve_deployment_connection()          (both mechanisms, unchanged)
        │
        ▼
 personal_local_mechanism(dir, service, bucket)
        │
        ├── ssh inputs present ────────────▶ scp + ssh extract into the VM (unchanged)
        │
        ├── bucketfs mapping resolves ─────▶ extract into the deployment's shared host directory
        │
        └── neither ───────────────────────▶ fail, naming both mechanisms' prerequisites
        │
        ▼
 wait for reconciliation, then ALTER SYSTEM SET SCRIPT_LANGUAGES   (both mechanisms, unchanged)
```

`personal_local_mechanism` is the only place that knows how the choice is made. Placement is the only step that varies. Connection resolution, the reconciliation wait, the entry assembly and the registration stay one code path, so the two mechanisms cannot drift apart in what they register.

The deployment directory decides, because each mechanism's inputs live in it. The SSH mechanism needs a port in the descriptor and a key file. The shared-directory mechanism needs a BucketFS mapping that resolves a host directory for the requested pair. The rule reads the literal input each step consumes, never a proxy for it.

`deployment_bucketfs_dir` owns the bucket directory. It reads the deployment's own BucketFS mapping rather than rebuilding the layout from a hard-coded path. An unknown service and bucket pair therefore fails instead of creating a directory the engine never reconciles.

The install removes `<slc-name>/` under that bucket directory and nothing else. The same bucket carries the operator's own UDF artifacts, so removing the directory itself would destroy them. That destination sits on the operator's host rather than inside a throwaway VM, so three guards stand before the `rm -rf`. Each of `--bfs-service`, `--bucket` and `--slc-name` must be a single path segment. The bucket directory must resolve inside the deployment directory. The removed path must resolve to a direct child of the bucket directory.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| One committed list, two readers | `dist/slc-sandbox-skeleton.txt`, Dockerfile, `dist/tests/slc_tarball_test.sh` | The library-surface file already sets this precedent. Two hand-maintained copies drift. |
| Vary the placement step, share everything after it | `scripts/install.sh` local branch | One registration path means one entry format and one idempotence rule for both mechanisms. |
| Detect the input a step consumes, not a version | `personal_local_mechanism` | A version string is a proxy. The deployment directory publishes the inputs themselves. |
| Read the deployment's own mapping, do not restate it | `deployment_bucketfs_dir` | The deployment declares which host directory serves each bucket. A hard-coded layout is a second copy of that declaration. |
| Resolve and check the destination before removing it | `require_path_segment`, `deployment_bucketfs_dir`, `extract_slc_into_shared_bucketfs` | The extract removes its destination on the operator's own machine. A name component that walks out of the bucket directory is a bug to refuse, not to perform. |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| The deployment directory's own contents choose the mechanism | Compare Personal version strings, rejected by the user. Probe the `exasol` launcher for a subcommand, rejected because the install calls no launcher command | Each mechanism's prerequisite is a file or a field in that directory. Reading it answers the question directly. |
| The SSH mechanism wins whenever both its inputs are present | Prefer the shared directory whenever the mapping resolves | No deployment that installs successfully today changes its mechanism. The shared-directory mechanism covers exactly the deployments that fail today. |
| Both mechanisms register through the install script's own `ALTER SYSTEM` | Hand placement and registration to another tool | Every install flag keeps one meaning across local, cloud and cluster installs. The script also stays the single owner of the entry format. |
| The destination comes from the deployment's BucketFS mapping file | Hard-code `local/runtime/vm-shared/exa/bucketfs/<service>/<bucket>` | A wrong `--bucket` then fails instead of silently extracting into a directory the engine never reconciles. |
| Ship all 21 verified directories, not a minimal subset | Ship only `proc`, the first failure | The issue records the failure walking from `proc` to `var/tmp` to `buckets`, one directory at a time. |
| Keep `dist/` as the home of the skeleton file | `crates/cargo-exasol-udf/`, beside the library surface | Only the packaging build and its test read the skeleton. `cargo exasol-udf validate` reads the library surface. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/slim-image | CHANGED | `specs/_plans/fix-personal-slc-install/container/slim-image/spec.md` |
| container/personal-install | CHANGED | `specs/_plans/fix-personal-slc-install/container/personal-install/spec.md` |
| container/personal-install-local | CHANGED | `specs/_plans/fix-personal-slc-install/container/personal-install-local/spec.md` |

## Impact

Rust UDFs start working on an Exasol Personal deployment that mounts the SLC read-only, where every call previously returned `22002 VM crashed`. Operators on that deployment must reinstall the container to pick up the fix.

`scripts/install.sh --deployment <name>` starts working against a local Personal deployment that publishes no SSH port and no node key. Every existing flag keeps its current meaning on both local mechanisms: `--bucket`, `--bfs-service`, `--slc-name`, `--host`, `--port`, `--user` and `--password` all apply, and `--scope` stays forced to `SYSTEM` on the local path as it is today. `--ssh-user` applies to the SSH mechanism only, which the help text states.

No deployment that installs successfully today changes its mechanism, because the SSH mechanism keeps priority wherever its inputs are present. The registration entry is identical on both mechanisms, so an operator who switches deployments sees the same `RUST=` value and the same `/buckets/<service>/<bucket>/<slc-name>/` path.

On the shared-directory mechanism the install writes inside the deployment directory. It removes and recreates the `<slc-name>` directory under the host directory the deployment's BucketFS mapping names, and it prints that path before removing anything. Everything else under that host directory is untouched, so an operator's own `udf/*.so` artifacts in the same bucket survive the install. The run needs `tar` on the invoking host and no SSH client.

`--bfs-service`, `--bucket` and `--slc-name` now reject a value that is not a single path segment. A value containing `/`, equal to `.` or `..`, or starting with `-` fails at argument validation instead of widening the removal.

An existing `RUST=` entry is replaced rather than duplicated, so a deployment that already carries a Rust language from another tool ends with one entry naming the bucket path this script installed to.

Breaking change for downstream UDF authors: the container fix bumps `[workspace.package].version` to `0.28.1`. The version is part of the ABI fingerprint, so every downstream UDF must be rebuilt against `0.28.1` before it loads in the new container.

## Dependencies

None new for the container. The shared-directory mechanism adds one host command, `tar`, to the local install path, which already needs `jq` and `exapump`. The SSH mechanism keeps the `ssh` and `scp` clients it needs today.

## Implementation Tasks

1. **Sandbox mount-point skeleton (Defect A)**
   1. Add `dist/slc-sandbox-skeleton.txt` listing one directory per line and nothing else: `boot`, `buckets`, `conf`, `dev`, `home`, `media`, `mnt`, `opt`, `proc`, `root`, `run`, `scripts`, `srv`, `sys`, `var/cache`, `var/lib`, `var/local`, `var/log`, `var/opt`, `var/spool`, `var/tmp`. Carry no comment lines, matching `crates/cargo-exasol-udf/slc-library-surface.txt`, whose readers parse one bare name per line. Put the rationale in the Dockerfile comment and in the contract test's header block instead.
   2. Add a failing assertion `slc_tarball_ships_sandbox_skeleton` to `dist/tests/slc_tarball_test.sh`: read every name from `dist/slc-sandbox-skeleton.txt`, require each to be a directory in the extracted tree, and require the skeleton to hold no regular file, symbolic link, device node or socket. Register it in the runner list at the end of the file, and name the committed file in a constant beside `LIBRARY_SURFACE_FILE`.
   3. In the `Dockerfile` staging stage, add `COPY dist/slc-sandbox-skeleton.txt /slc-meta/sandbox-skeleton` and a `RUN` that creates every listed directory under `/slc`. Place both after the existing `RUN mkdir -p /slc/exaudf /slc/build_info` and before the `RUN tar` step. Comment the `RUN` with what the skeleton is for and what fails without it.
   4. Build the tarball and run `bash dist/tests/slc_tarball_test.sh` against it. Confirm `slc_tarball_staged_surface_within_ceiling` still passes and record its reported measured size in the verification report.
   5. Bump `[workspace.package].version` and the pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` to `0.28.1`, regenerate `Cargo.lock`, and commit the lockfile in the same change. Bump once for the whole plan, not once per defect.

2. **Shared-directory placement (Defect B)**
   1. Add two failing tests to `scripts/tests/install-personal-test.sh`. `rejects_path_unsafe_name_components` drives `require_path_segment` and asserts it rejects an empty value, a value containing `/`, the values `.` and `..`, and a value starting with `-`, and accepts a plain segment such as `rustslc`. `resolves_shared_bucketfs_dir_from_mapping` builds a deployment-directory fixture holding `local/runtime/vm-shared/exa/bucketfs.conf` with the two mapping lines a live deployment carries, one `__builtin__` line and one `bfsdefault default` line. It asserts the resolved host directory for `bfsdefault`/`default`, a failure for a service and bucket pair no line names, a failure for a deployment with no mapping file, a failure for a mapping line whose host directory does not exist, and a failure for a mapping line whose path resolves outside the deployment directory. It also asserts the exit status each of those four failure fixtures returns: 2 for the absent mapping file, 3 for the unmatched pair, 1 for the absent host directory, and 1 for the path resolving outside the deployment directory.
   2. Add `require_path_segment <flag> <value>`, which accepts only a single path segment: non-empty, free of `/`, not `.` or `..`, and not starting with `-`. It prints a clear error naming the flag and returns 1 rather than calling `die`, so the test harness can drive it in-process. Call it from the existing non-empty validation block in `scripts/install.sh` for `--bfs-service`, `--bucket` and `--slc-name`, replacing the three `-z` checks there with `require_path_segment … || exit 1`. All three values are interpolated into a path the install removes with `rm -rf`. A component carrying `/` or `..` walks out of that subtree into the deployment's own data.
   3. Add `deployment_bucketfs_dir <dir> <service> <bucket>`, which prints the host directory that serves that pair. Read the mapping file above. Match the line whose service and bucket fields equal the arguments. Join the VM-side path that line names to `<dir>/local/runtime/vm-shared`. Compare physically resolved paths for the inside-`<dir>` check (`cd … && pwd -P`, or `realpath`), never a string prefix on the unresolved join. Every failure prints a clear error and returns one of the three exit statuses below. [expert]
      - Status 2: the mapping file itself is absent.
      - Status 3: the file names no line for the requested pair. Name the pairs the mapping does serve.
      - Status 1: the joined path is not an existing directory. Name that resolved directory.
      - Status 1: the joined path does not resolve inside `<dir>`. Name that resolved directory.
   4. Add a failing test `extracts_slc_into_shared_bucketfs`. Build a small tarball carrying `exaudf/exaudfclient` with the executable bit. Seed the fixture bucket directory with a stale file under `<slc-name>/` and a sibling file outside it at `udf/libother.so`. Extract twice, and assert the stale file is gone, the tree is present once, `udf/libother.so` is still present after both extractions, a tarball without an executable `exaudf/exaudfclient` fails the call, and an `<slc-name>` that is not a direct child of the bucket directory fails before anything is removed.
   5. Add `extract_slc_into_shared_bucketfs <dir> <tarball>`. Resolve the bucket directory through `deployment_bucketfs_dir`, then join `<slc-name>` to it to form the destination. Refuse unless that destination is a direct child of the bucket directory: its parent, physically resolved, MUST equal the physically resolved bucket directory, and its basename MUST equal `<slc-name>`. Print the joined destination, then remove only it, recreate it, extract the tarball into it, and confirm `exaudf/exaudfclient` is executable. Never remove or recreate the bucket directory itself, which also holds the operator's own `udf/*.so` files. Mirror the step sequence `extract_slc_into_bucketfs` already runs inside the VM, so both mechanisms leave the same tree. [expert]

3. **Local mechanism selection and dispatch (Defect B)**
   1. Add a failing test `selects_local_mechanism_from_deployment_directory` to `scripts/tests/install-personal-test.sh`, covering four fixtures: both SSH inputs present; neither SSH input present with a mapping that resolves the requested pair; neither present with no mapping file; and neither present with a mapping file that names no line for the requested pair. Assert `ssh`, `shared`, a failing exit whose message names both mechanisms' prerequisites, and a failing exit whose message names the requested service and bucket pair and the pairs the mapping does serve.
   2. Add `deployment_supports_ssh_transport <dir>`, which returns success only when `.connection.sshPort` reads as a port number and `local/node_access.pem` is readable. Reuse `deployment_ssh_port` and `deployment_key_path`.
   3. Add `personal_local_mechanism <dir> <service> <bucket>`. It prints `ssh` when `deployment_supports_ssh_transport` succeeds, and `shared` when `deployment_bucketfs_dir` resolves a directory. It is the only reader of those two predicates, and it reads no version string. It branches the remaining cases on `deployment_bucketfs_dir`'s three failure statuses:
      - Status 2: fail with one error naming both mechanisms' prerequisites.
      - Status 3: fail naming the requested service and bucket pair, and the pairs the mapping does serve.
      - Status 1: fail naming the resolved host directory and why it is unusable. Do not report the flags as wrong, because a host directory the deployment has not created yet is a normal state.
   4. Add a failing test `local_mechanisms_share_the_registration_inputs`, which runs the mechanism selection against an SSH fixture and a shared-directory fixture and asserts both yield the same resolved endpoint from `resolve_deployment_connection` and the same string from `script_languages_entry`.
   5. Rewire the `"local"` branch of `main`. Keep connection resolution, the `SYSTEM` scope, the reconciliation wait and the `ALTER SYSTEM` step as they are, for both mechanisms. Call `personal_local_mechanism` after connection resolution and dispatch the placement step alone: `extract_slc_into_bucketfs` on `ssh`, `extract_slc_into_shared_bucketfs` on `shared`. Read the SSH port and the node key inside the `ssh` branch only, so a deployment without them no longer fails before the dispatch.
   6. Update the `usage()` text and the `--deployment` help so both mechanisms and their prerequisites appear, so `--ssh-user` states that it applies to the SSH mechanism only, and so the tool list names `tar` for the shared-directory mechanism and `ssh`/`scp` for the SSH mechanism.

4. **Documentation**
   1. Update `docs/installation.md`'s Exasol Personal section: state the two local mechanisms, the rule that chooses between them, and that every other option and the registration step behave the same on both. For a deployment that publishes no SSH inputs, document copying a UDF `.so` with `mkdir -p` and `cp` into `~/.exasol/personal/deployments/<name>/local/runtime/vm-shared/exa/bucketfs/<service>/<bucket>/udf/`, the host directory the deployment's BucketFS mapping serves at `/buckets/<service>/<bucket>/udf/`. Keep the existing `sshPort` and `node_access.pem` snippet beside it for a deployment that still publishes SSH inputs. Keep the section within a few lines of its current length.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: SLC sandbox skeleton | 1.1-1.5 | — | spec delta `container/slim-image`; `dist/slc-sandbox-skeleton.txt`, `Dockerfile`, `dist/tests/slc_tarball_test.sh`, `Cargo.toml`, `Cargo.lock` |
| B: Personal local install mechanism | 2.1-2.5, 3.1-3.6, 4.1 | — | spec deltas `container/personal-install`, `container/personal-install-local`; `scripts/install.sh`, `scripts/lib/script_languages.sh`, `scripts/tests/install-personal-test.sh`, `docs/installation.md` |

Group A and group B share no source file, no test file and no spec delta. Task 1.5 is the only edit to `Cargo.toml` and `Cargo.lock` in this plan. Tasks 2.1-2.5 and 3.1-3.6 stay in one group because they edit the same two files.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None | — | The SSH mechanism stays reachable for a deployment directory that still carries `.connection.sshPort` and `local/node_access.pem`. `deployment_ssh_port`, `deployment_key_path`, `extract_slc_into_bucketfs` and `VM_BUCKETFS_ROOT` all keep a caller. |

Carrying both mechanisms is a deliberate cost with a scheduled end. File a `feature`-labelled GitHub issue in this plan's PR to delete the SSH mechanism, `deployment_ssh_port`, `deployment_key_path`, `extract_slc_into_bucketfs`, `--ssh-user`, and their tests once no supported local Personal deployment publishes SSH inputs. Without that issue the dead branch outlives the deployments it serves.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| slim-image: SLC tarball ships the sandbox mount-point skeleton | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_ships_sandbox_skeleton` |
| personal-install: Connection details are read fresh on every run | Integration | `scripts/tests/install-personal-test.sh` | `reads_ssh_port_from_deployment_json` |
| personal-install: SLC is deployed via filesystem BucketFS reconciliation | Manual | `docs/installation.md` Personal section | Manual run against a live local deployment, once per mechanism |
| personal-install: Registration targets the exaudfclient executable | Integration | `scripts/tests/install-personal-test.sh` | `fragment_points_at_executable_no_leading_slash`, `local_mechanisms_share_the_registration_inputs` |
| personal-install: Registration is system-scoped and preserves existing entries | Integration | `scripts/tests/install-personal-test.sh` | `preserves_existing_script_languages` |
| personal-install: A registered Rust UDF executes on Personal | Manual | `docs/installation.md` Personal section | Manual run, both mechanisms |
| personal-install: Deployment backend selects the transport | Integration | `scripts/tests/install-personal-test.sh` | `selects_transport_from_backend` |
| personal-install-local: Local install resolves the DB password from the deployment directory | Integration | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor` |
| personal-install-local: Local connection details resolve from the deployment directory | Integration | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor` |
| personal-install-local: Command-line flags override descriptor-derived local values | Integration | `scripts/tests/install-personal-test.sh` | `cli_port_overrides_local_descriptor`, `cli_host_overrides_local_descriptor` |
| personal-install-local: A local descriptor that omits the SQL port is reported | Integration | `scripts/tests/install-personal-test.sh` | `resolves_local_defaults_when_db_port_absent` |
| personal-install-local: The deployment directory selects the local install mechanism | Integration | `scripts/tests/install-personal-test.sh` | `selects_local_mechanism_from_deployment_directory` |
| personal-install-local: The shared-directory mechanism extracts into the deployment's own BucketFS directory | Integration | `scripts/tests/install-personal-test.sh` | `extracts_slc_into_shared_bucketfs` |
| personal-install-local: The shared-directory destination is checked before anything is removed | Integration | `scripts/tests/install-personal-test.sh` | `rejects_path_unsafe_name_components`, `resolves_shared_bucketfs_dir_from_mapping`, `extracts_slc_into_shared_bucketfs` |
| personal-install-local: Both local mechanisms register through the install script | Integration | `scripts/tests/install-personal-test.sh` | `local_mechanisms_share_the_registration_inputs` |
| personal-install-local: Re-running the local install replaces the installed SLC | Integration | `scripts/tests/install-personal-test.sh` | `extracts_slc_into_shared_bucketfs`, `preserves_existing_script_languages` |

`dist/tests/slc_tarball_test.sh` needs `readelf`, `du -sb` and `stat -c`, so it runs on the Linux CI runner, not on a macOS workstation. `scripts/tests/install-personal-test.sh` runs everywhere and is already wired into CI.

Personal is not exercisable in CI: no arm64 Exasol database image exists. The two scenarios above marked Manual keep the existing manual-verification rule that `container/personal-install` already records.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/slim-image | `docker build --target artifact --output type=local,dest=/tmp/lc-out .` then `tar -tzf /tmp/lc-out/lc-rs.tar.gz \| grep -E '^\./(proc\|buckets\|var/tmp)/$'` | Three matching directory entries |
| container/slim-image | `bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` (Linux) | `All tests passed`, with the reported staged surface below `24000000` bytes |
| container/personal-install-local | `scripts/install.sh --deployment default` against a deployment whose descriptor carries no `.connection.sshPort` | The run reports the shared-directory mechanism, prints the destination it replaces, and ends by printing the registered bucket path |
| container/personal-install-local | `ls ~/.exasol/personal/deployments/default/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default/rustslc/exaudf/exaudfclient` | The executable is present on the host after the run |
| container/personal-install-local | Run the same install a second time with a rebuilt tarball | The run completes, the bucket holds one `rustslc` tree, and it carries the rebuilt tarball's contents |
| container/personal-install-local | `scripts/install.sh --deployment default --bucket nosuchbucket` | The run fails naming the service and bucket pair the deployment's BucketFS mapping does not serve, and removes nothing |
| container/personal-install | `exapump sql "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'" -d "exasol://sys:<pw>@127.0.0.1:8563?validateservercertificate=0"` | The value keeps every pre-existing language and carries exactly one `RUST=` entry naming `/buckets/bfsdefault/default/rustslc/` |
| container/personal-install | Create the scalar UDF from `docs/writing-a-udf.md`, then `SELECT health_check()` | `ok`, instead of `VM error: Internal error: VM crashed (SQL state: 22002)` |
| container/personal-install-local | `scripts/install.sh --deployment <name>` against a deployment that still carries `.connection.sshPort` and `local/node_access.pem` | The run reports the SSH mechanism and registers over the resolved endpoint, unchanged from today |
| container/personal-install-local | The same command against a deployment directory with neither the SSH inputs nor a BucketFS mapping | The run fails naming both mechanisms' prerequisites |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Personal install unit tests | `bash scripts/tests/install-personal-test.sh` | No `FAIL` line |
| SLC tarball contract | `bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` | `All tests passed` |
| Lint | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` | 0 errors and 0 warnings |
| Lockfile | `git diff --name-only \| grep Cargo.lock` | `Cargo.lock` present in the change |
