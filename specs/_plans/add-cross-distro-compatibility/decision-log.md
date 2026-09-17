# Decision Log: add-cross-distro-compatibility

## Interview

Planning ran headless. One asynchronous exchange with the human took place, through the orchestrator.

**Q:** Issue #110 (a Podman custom-SLC install crash on Exasol Personal) looks unrelated to the E2E and distribution-compatibility request. How should planning proceed?
**A:** Both are related, plan for both.

**Q (implied by the request text, answered by research rather than by the human):** Can an end-to-end test run on a GitHub Actions macOS runner?
**A:** Resolved by research, not escalated. See decision [8].

The remaining request text, taken verbatim from the user: "I want an E2E for Suse and Fedora as well as the Rust SLC must be compatible with all Linux supported by Exasol. Do a research first; also: can we do an E2E on macOS via GH actions?"

## Design Decisions

### [1] The sandbox directory skeleton, not the install method, is the root cause of issue #110

- **Decision:** Fix issue #110 in this repository by shipping the directory skeleton the Exasol UDF sandbox expects inside the SLC root. Treat the reporter's `podman import` versus `podman pull` hypothesis as disproved.
- **Alternatives:** Ship an OCI archive so the custom install can use `podman load`. Rejected: `exasol slc custom install` validates and accepts a flat root-filesystem tarball only, so no other format reaches that code path. Ask Exasol Personal to mount the SLC writable. Rejected: `internal/localinstall/podman_install.go` attaches every SLC with `--mount type=image`, which is read-only by default, through one code path shared by catalog and custom SLCs, so the mount mode is not what distinguishes a working SLC.
- **Rationale:** Evidence, in three parts. Every official flavor Dockerfile in `exasol/script-languages-release` runs `mkdir /conf /buckets` on top of a full Ubuntu base, so an official SLC root already contains every directory the sandbox creates. `podman import` produces an ordinary image and a container from it gets a writable layer exactly as a pulled image does, so the import path removes no writability. The reporter's own progression `proc` then `var/tmp` then `buckets` converges on that official layout rather than diverging. This project's `FROM scratch` tree is the outlier.
- **Promotes to ADR:** yes

### [2] Derive the skeleton from the official SLC layout, not from the observed failures

- **Decision:** `dist/slc-sandbox-skeleton.txt` names every top-level directory an official SLC root provides, plus `conf`, `buckets` and `var/tmp`. The file's header records which entries are observed failures and which come from the official layout.
- **Alternatives:** Add only `proc`, `var/tmp` and `buckets`, the three paths observed to fail. Rejected: that set was found one failure at a time and has no known end, because Nano is closed source and no Exasol document enumerates the mount set.
- **Rationale:** An empty directory costs no bytes and no library surface, so matching a published layout is cheaper than another round of iteration on a user's deployment.
- **Promotes to ADR:** no

### [3] The SLC publishes its builder rustc identity as a committed record

- **Decision:** Add `crates/cargo-exasol-udf/slc-builder.txt` holding the builder image reference and the `rustc --version` identity it provides. The container build verifies both against its own builder stage. `validate` compares an artifact's baked identity against the record.
- **Alternatives:** Keep comparing an artifact against the CLI's own `EXA_SDK_FINGERPRINT`. Rejected: `cargo install cargo-exasol-udf` bakes the author's rustc into the CLI, so that comparison passes on every host and detects no skew against the container. Relax the runtime's fingerprint check to a version prefix. Rejected: the check exists to stop toolchain-mismatch undefined behaviour, and loosening it trades a clear error for silent risk.
- **Rationale:** The loader compares the fingerprint by exact string equality, so an artifact loads only when its rustc identity equals the SLC builder's. Without a published record, that requirement is unknowable to an author and undetectable before `dlopen`. This is the same pattern `slc-glibc-floor.txt` already uses: one committed value, verified by the build that produces it.
- **Promotes to ADR:** yes

### [4] `cargo exasol-udf build --container` is the portable build path

- **Decision:** Add an explicit `--container` flag that runs the release build inside the recorded builder image, writing to `target/slc/release/`.
- **Alternatives:** Switch to a container build automatically when the host is unfit. Rejected: an implicit container build changes build time, disk use and network use without the author asking, and it fails on a host with no engine that would otherwise have produced a usable artifact. Add a `cargo-zigbuild` dependency to target a fixed glibc version. Rejected: it fixes the glibc half only, and the rustc identity half would still fail at load. Document a `docker run` recipe instead of a flag. Rejected: the CLI already owns the build, so handing the author a recipe leaves the deepest part of the job outside the tool.
- **Rationale:** Only a build inside the recorded image satisfies both the glibc floor and the rustc identity by construction. One flag on the subcommand that already owns building keeps the common path unchanged.
- **Promotes to ADR:** yes

### [5] No `--builder-image` override

- **Decision:** `--container` reads the builder image reference from the record and accepts no override.
- **Alternatives:** Expose `--builder-image <ref>` for mirrored or air-gapped registries. Rejected for now.
- **Rationale:** The record is the contract, and a flag that lets an author build against a different image reintroduces exactly the mismatch the flag exists to prevent. An operator behind a mirror retags the recorded reference locally, which needs no CLI surface.
- **Promotes to ADR:** no

### [6] Record location follows which component reads the record

- **Decision:** `slc-builder.txt` joins `slc-glibc-floor.txt` and `slc-library-surface.txt` in `crates/cargo-exasol-udf/`. `slc-sandbox-skeleton.txt` and `supported-author-hosts.txt` go in `dist/`.
- **Alternatives:** One `slc-platform.toml` holding every value. Rejected: the `debian:trixie-slim` staging stage has no TOML parser and reads these records with `cat` and a shell loop, so a structured format would need a parser inside a stage that deliberately carries none. Put every record in one directory. Rejected: it would either place a file the CLI never reads inside the CLI crate, or move a file out of the `include_str!` path it depends on.
- **Rationale:** The CLI compiles its records in with `include_str!`, which decides where they live. Nothing else imposes a location, so the packaging records sit with the packaging tests that read them.
- **Promotes to ADR:** no

### [7] The author-host distribution matrix covers seven rows, chosen from Exasol's own published requirements

- **Decision:** Verify `ubuntu:22.04`, `ubuntu:24.04`, `almalinux:8`, `almalinux:9`, `debian:13`, `opensuse/leap:16.0` and `fedora:44`. Each image is one row of `dist/supported-author-hosts.txt`, keyed by the pair `<family> <version>`. The two Ubuntu images are the rows `ubuntu 22.04` and `ubuntu 24.04`, and the two AlmaLinux images are the rows `rhel 8` and `rhel 9`, because AlmaLinux stands in for RHEL. Seven rows produce seven legs, and macOS adds an eighth row that carries the placeholder image reference and contributes no leg.
- **Alternatives:** Verify only openSUSE and Fedora, the two families named in the request. Rejected: the request's second half asks for compatibility with every Linux Exasol supports, and neither openSUSE nor Fedora appears on Exasol's database-host list. Verify every in-support version of every family. Rejected: twelve or more legs for no new failure mode, since the glibc version is what varies and the families repeat it.
- **Rationale:** Exasol's system requirements page names Ubuntu 20.04, 22.04 and 24.04 LTS and RHEL 8, 9 and 10 as supported database hosts, and the driver pages additionally name openSUSE 15 and Debian 11 and 12. AlmaLinux stands in for RHEL, which needs no subscription to pull. Ubuntu 20.04 is excluded because it is in extended security maintenance. Fedora is on no Exasol list but is the request's explicit ask and the only mainstream image whose glibc is above the floor, which makes it the canary for the container-only build path. Debian 13 sits exactly at the floor and is the SLC's own donor. This matrix covers the author-host claim only. The database-host claim gets its own matrix, per decisions [13] and [14].
- **Promotes to ADR:** no

### [8] No Exasol end-to-end test on a GitHub Actions macOS runner

- **Decision:** Do not add a macOS end-to-end job. Document why, and make macOS a supported author host through `--container` instead.
- **Alternatives:** Colima or Lima on an Intel macOS label (`macos-15-intel`, `macos-15-large`). Rejected. A self-hosted Intel Mac runner, or a third-party macOS provider with nested virtualization. Rejected as disproportionate to the coverage gained.
- **Rationale:** Three independent blockers, any one of them decisive. The first two concern runner capability. GitHub documents that arm64 macOS runners support no nested virtualization, because of Apple's Virtualization Framework, so every Docker, Colima, Lima and Podman path fails there. GitHub documents that job and service containers require a Linux runner, on every macOS label. No cross-job networking exists that would let a macOS job reach a database running in a Linux job.

  The third concerns the database image. `exasol/docker-db` publishes amd64 manifests only and states that only Docker on Linux is supported. An arm64 runner therefore could not run the database even with nested virtualization. The remaining Intel labels have an announced end of life and are billed even on public repositories. Their 14 GB disk is not shown to fit a 3.2 GB compressed image plus a Linux VM disk plus the database's data volume. The useful macOS outcome is the author path, not the server path: a macOS host cannot emit a Linux `.so` natively, and `--container` gives it one.
- **Promotes to ADR:** yes

### [9] The Fedora end-to-end path is the container-built artifact

- **Decision:** Run the openSUSE host-built artifact and the Fedora container-built artifact through the live-database suite.
- **Alternatives:** Run a Fedora host-built artifact end to end. Rejected: a Fedora host build references glibc symbol versions above the floor, so it cannot load, and asserting that it loads would encode a failure.
- **Rationale:** An end-to-end test should exercise the path a Fedora author actually uses. That path is `--container`. The host-built Fedora artifact is still covered, as the case whose rejection message must name the remedy.
- **Promotes to ADR:** no

### [10] Test the real container build in the distribution harness, not in a Rust integration test

- **Decision:** `crates/cargo-exasol-udf/tests/build.rs` asserts engine selection and argument assembly against a recording stub engine on `PATH`. `dist/tests/distro_udf_build_test.sh` runs the real container build on every recorded image.
- **Alternatives:** A Rust integration test that runs a real `docker run`. Rejected: `cargo test` would then fail on a developer machine with no container engine, and the project's rule is that a test needing a container fails rather than skips, which would make the default test command unusable off CI.
- **Rationale:** The stub gives deterministic, fast coverage of the argument contract. The shell harness gives real coverage on seven distributions, which is stronger evidence than one local run.
- **Promotes to ADR:** no

### [11] Group F owns every CI file edit

- **Decision:** Group F is the single owner of `.github/workflows/ci.yml` and `.github/actions/exasol-it/action.yml`. Groups A through E add records, code and tests but wire nothing into CI. Group F does all CI wiring and the live-database scenarios, and depends on A, B, C and D.
- **Alternatives:** Let each group wire its own CI job. Rejected: five groups writing one file is a merge conflict, not parallelism.
- **Rationale:** The workflow file is a single shared resource with no natural split, so it gets a single owner and a declared dependency instead.
- **Promotes to ADR:** no

### [12] `scripts/install.sh` detects the SSH-less local deployment by descriptor, not by host operating system

- **Decision:** Treat a local deployment as SSH-less when `deployment.json` carries no `connection.sshPort` or `local/node_access.pem` is absent.
- **Alternatives:** Branch on the host operating system, treating Linux as the container-engine shape and macOS as the VM shape. Rejected: Exasol Personal's own release notes state that macOS now runs the same Podman installation inside a managed VM, so the host operating system does not determine the deployment shape and would become wrong without warning.
- **Rationale:** The descriptor is the only fact available on every host, and Exasol Personal's own specification states that a local deployment's `connection` block carries no SSH endpoint.
- **Promotes to ADR:** no

### [13] The Exasol-supported Linux list constrains both the author host and the database host

- **Decision:** Read "the Rust SLC must be compatible with all Linux supported by Exasol" as two claims, and cover both. The author-build claim says a Rust UDF author can build on Ubuntu, RHEL, Debian, openSUSE, Fedora and macOS, through a plain host build where the host fits the record and through `--container` otherwise. The host-runtime claim says a registered SLC loads and executes a UDF when Exasol itself runs on a host of each of those Linux families. The two claims are published separately, recorded separately, and verified by different evidence.
- **Alternatives:** Read the request as the author-build claim alone, which is what round 1 flagged the plan as having silently done. Rejected: the SLC carrying its own root filesystem makes the database host's *userland* irrelevant, but not its *kernel*, so the reasoning that would justify dropping the runtime claim does not hold. Read it as the host-runtime claim alone. Rejected: the request also asks for openSUSE and Fedora end-to-end coverage, and neither is on Exasol's database-host list, so an author-host reading is the only one that places them.
- **Rationale:** The mechanism that makes the runtime claim host-dependent is the host kernel's security module, not the SLC image. The engine runs a UDF through `nschroot`, which needs `CAP_SYS_ADMIN` in an unprivileged user namespace. AppArmor grants or denies it on Debian, Ubuntu and openSUSE, and SELinux does the same on RHEL and Fedora. This project already meets that denial on its own Ubuntu CI hosts, where it reports as `22002 VM crashed` and is fixed by a host `sysctl`, with the cause visible only in the kernel audit log. That failure is independent of everything the SLC image contains, so no image-level evidence can stand in for it.
- **Consequence for verification:** evidence must come from a run on a host of each family, because a distribution container on an Ubuntu-kernel runner exercises the runner's module and not the distribution's. GitHub hosts Ubuntu, Windows and macOS runners only, so no hosted label carries a SUSE, Fedora or RHEL kernel. The matrix therefore runs on self-hosted runners for openSUSE, Fedora, RHEL and Debian, addressed only by the generic label triple `self-hosted, linux, <family>`. Ubuntu needs no new runner: the existing `integration` job is that family's leg, and it is extended rather than duplicated.
- **Confidentiality constraint:** this repository is public. No account, token, endpoint, host name or other provisioning detail for those runners appears in this plan, in any spec delta, or in any workflow file. The workflow assumes a label exists and is reachable, and nothing more. Provisioning and registering the runners is infrastructure work outside this repository's diff, recorded as a Dependency in `plan.md`.
- **Promotes to ADR:** yes

### [14] The host prerequisite each database-host family needs is a committed record

- **Decision:** Add `dist/supported-database-hosts.txt`, holding one line per family version with its kernel security module and the host prerequisite a UDF needs on it. The row key is the pair `<family> <version>`, matching the author-host record, because `kernel.apparmor_restrict_unprivileged_userns` is an Ubuntu 24.04 kernel setting rather than a family-wide one. Every live-database run applies exactly what that record holds for its row and makes no other host change. The user documentation and the workflow matrix are both checked against it.
- **Alternatives:** Keep the Ubuntu `sysctl` inline in the workflow, as today, and add one such step per family. Rejected: the current inline step is already an operator prerequisite that exists only inside a CI comment, so an operator installing Exasol on Ubuntu 24.04 rediscovers it as `22002 VM crashed`. Publish the prerequisites in documentation only, with no record. Rejected: a documented prerequisite with no machine-readable source drifts from what the runs actually apply, which is the failure the plan's other four records exist to prevent.
- **Rationale:** The prerequisite is the deliverable, not a CI detail. Recording it makes it publishable, makes the documentation checkable against it, and makes a run that needed an undocumented extra step fail rather than pass quietly. The SELinux entries are not yet known and are established by the first run on an enforcing host of each family. The record is where that answer lands.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] The compatibility request covers the database host as well as the author host

- **Finding:** round 1 `[INTENT_DRIFT]` BLOCKER. The request names Exasol's supported Linux list, which is a list of database hosts, and the plan converted it into a claim about author hosts without recording the substitution as a decision. Every verification leg built a UDF on a distribution; none ran the SLC against an Exasol on one.
- **Direction change:** the human resolved the question, and the resolution widened the scope rather than confirming the reviewer's proposed narrowing. Round 1's `Fix:` line asked for a decision titled "The Exasol-supported Linux list constrains the author host, not the SLC runtime"; that fix is superseded. Both readings now apply. Decision [13] records the reconciliation, the host-kernel mechanism, the hosted-runner limitation and the self-hosted-runner resolution with its confidentiality constraint. Decision [14] records the prerequisite record. A new feature `container/host-runtime-compatibility` carries the runtime claim, task group 4 publishes its record and documentation, and task group 6 adds the `host-runtime` job and the shared bring-up action. `plan.md` gains the mechanism in Context, the claim in Goals, the second path in the architecture diagram, three Consequences rows, a Residual risk paragraph, a Dependencies entry for the runners, four Verification scenario rows and four Manual Testing rows.
- **Promotes to ADR:** yes

### [plan-review] validate must accept the artifact build --container produces

- **Finding:** round 1 `[REQUIREMENT_CONFLICT]` BLOCKER. The new `build --container` scenario required the artifact to pass `validate` on a host whose rustc differs from the SLC builder's, while the recorded validate scenarios required the whole `sdk_fingerprint` to match the CLI's own baked value. `crates/cargo-exasol-udf/src/validate.rs:146` implements exactly that whole-string comparison, and no task changed it, so the plan shipped a contradiction.
- **Direction change:** the fingerprint is compared in two parts against two sources. The SDK-version part is compared against the CLI's own `EXA_SDK_FINGERPRINT`, and the rustc-identity part against `crates/cargo-exasol-udf/slc-builder.txt`. The `tools/cargo-exaudf-validate` delta now wraps "validate rejects an ABI or fingerprint mismatch" in a CHANGED block stating the split and forbidding the whole-string comparison, and "validate accepts a compatible .so" names the SDK-version part only. Task 2.2 implements the split and carries `[expert]`. Task 2.5 and a Verification row add the case that a container-built artifact validated by a CLI compiled with a different rustc exits zero.
- **Promotes to ADR:** no

### [plan-review] macOS is listed in the author-host record with the container build path

- **Finding:** round 1 `[REQUIREMENT_CONFLICT]` BLOCKER. The documentation was to name `--container` for macOS, a drift test was to assert the documentation names exactly the recorded platforms, and macOS was in no record. macOS also had no scenario, no task and no test, and `validate` cannot run there at all.
- **Direction change:** the human chose to list macOS. It joins `dist/supported-author-hosts.txt` with the placeholder image reference and the `container` build path, and the record's rule now states that a platform naming no image must be container-only and contributes no harness leg. `tools/cargo-exaudf` gains the scenario "build --container serves a host that cannot load the artifact it produces", covered by `build_container_skips_local_validation_and_states_why` and by a macOS Manual Testing row. `tools/cargo-exaudf-validate` states in its Background and in its accept scenario that `validate` needs a Linux host.
- **Promotes to ADR:** no

### [plan-review] The distribution harness runs inside the container and uses one shared path

- **Finding:** round 1 `[HIDDEN_DEPENDENCY]` BLOCKER. A bind mount requested over a mounted Docker socket is resolved by the host daemon against the host filesystem, so a probe crate and a cargo cache created inside the distribution container would not exist for the sibling container. The plan also never said where `cargo-exasol-udf` comes from in each image, and the task and the checklist disagreed on who starts the container.
- **Direction change:** the human chose one shared path with the harness running inside the container. Task 2.11 now scaffolds the probe crate and points `CARGO_HOME` under the mounted repository path, so every path is identical inside and outside, and builds `cargo-exasol-udf` from the mounted workspace with that distribution's rustup toolchain. Task 6.1 and the checklist's author-host matrix row state one invocation contract: CI starts the container and execs `dist/tests/distro_udf_build_test.sh <family>` inside it, and the script never starts a container itself. The script's argument became a family rather than an image reference, since it no longer resolves an image.
- **Promotes to ADR:** no

### [plan-review] The author-host record's columns and its host-path rule are stated once

- **Finding:** round 1 `[COMPLETENESS_GAP]` BLOCKER, three defects in one requirement. The spec's stated fields and the task's recorded fields disagreed. The spec made glibc alone decide the host build path, contradicting the plan's own statement that a distribution-packaged Rust never matches the fingerprint. Task 2.10's toolchain install route was ambiguous between a distribution package and a rustup channel.
- **Direction change:** the record carries the four fields `<family> <image-reference> <glibc-version> <build-path>`, stated identically in task 3.1 and in the `container/slc-platform-contract` scenario. A `host` build path now requires both a glibc at or below the floor and a rustup-installed toolchain of the exact version the builder record names. A platform failing either condition is `container`. Task 2.11 names rustup and that exact version, and forbids the distribution's own Rust package.
- **Superseded by:** the round-2 entry "The author-host and database-host records are keyed by family and version". That entry adds a version field, so the record now carries five fields and the key is a pair.
- **Promotes to ADR:** no

### [plan-review] Advisory findings bundled with the blocker fixes

- **Finding:** round 1 raised thirteen ADVISORY findings. Those touching files already opened for the blocker fixes, and resolvable in a line or two, were applied in the same pass.
- **Direction change:** applied. `[SCOPE_CREEP]`: `plan.md` § Impact now names the concrete failure group 5 removes and distinguishes it from issue #110's own route. `[REQUIREMENT_CONFLICT]` on `container/personal-install`: added to § Features as CHANGED with a delta file and a CHANGED block on "Deployment backend selects the transport", plus task 5.3 and a Verification row. `[UNSTATED_ASSUMPTION]`: the architecture the container build inherits is now stated in the `tools/cargo-exaudf` scenario and in task 2.10. `[EFFORT_MISESTIMATION]`: § Dependencies states the `distro-build` cost and its effect on `integration`'s critical path, and task 6.1 states that it runs on every push. `[CLUSTER_INCOHERENCE]`: the author-host publication tasks became their own group C depending on B. `[TASK_GRANULARITY]`: fixtures moved to `dist/tests/fixtures/skeleton/`. `[AMBIGUOUS_REQUIREMENT]`: task 6.5 states that `distro_built_artifact_executes` is a helper reusing the existing `Harness`, and task 6.2 names the two distinct scaffold crate names. `[INFORMATION_LEAKAGE]`: the builder record holds the fingerprint's rustc part verbatim, read out of the compiled binary, leaving `crates/exasol-udf-sdk/build.rs` the only owner of that format. That was a precondition for the fingerprint-split fix, not optional polish. `[SHALLOW_DESIGN]`: `ARG BUILDER_IMAGE` is gone, replaced by a literal `FROM` plus `dist/tests/builder_record_test.sh`. `[PROSE_BLOAT]` and `[PROSE_UNCLEAR]`: decision [6]'s heading, decision [8]'s rationale and the `cargo-exaudf` Background sentence were rewritten. Two advisories were left for round 2 because their fixes exceed a line: the read-only chroot self-test extension, and the nested-path creation ordering in the skeleton record.
- **Promotes to ADR:** no

### [plan-review] The it-runner binary sets a glibc floor for every self-hosted database-host runner

- **Finding:** round 2 `[HIDDEN_DEPENDENCY]` BLOCKER. The `build` job compiles `it-runner` on `ubuntu-latest` and uploads it, and task 6.6 downloads that binary onto a self-hosted host of each family. A glibc-dynamic binary does not start on a host whose glibc is older than the one it was linked against, so a RHEL 8 or RHEL 9 runner, or an openSUSE Leap 15 runner, fails at the loader before any Exasol container exists. § Dependencies stated only the label triple, the host operating system and a container engine.
- **Direction change:** three changes. `dist/supported-database-hosts.txt` gains a version field, and task 4.1 names the exact version each family's claim covers, chosen so its glibc clears the floor. § Dependencies states that every `host-runtime` runner MUST provide a glibc at or above the glibc of the `ubuntu-latest` runner that compiles `it-runner`. Task 6.4 makes that comparison the composite action's first step, against a floor the `build` job records beside the `it-test` artifact, and fails naming both versions. § Residual risk names the in-support versions that sit below the floor and therefore carry no automated leg.
- **Promotes to ADR:** no

### [plan-review] Every live-database leg downloads the same artifact set

- **Finding:** round 2 `[HIDDEN_DEPENDENCY]` BLOCKER. `db_roundtrip_all_scenarios` is the whole of `it-runner`, and task 6.5 hangs `distro_built_artifact_executes` off it. Only the `integration` job downloaded the two distribution-built artifacts, and the `host-runtime` job had no `needs` on `distro-build`. Every `host-runtime` leg would run a test that reads two files its job never downloaded, which is the `No such file or directory` failure shape `CLAUDE.md` already records for the `test-udfs/*` allowlist.
- **Direction change:** task 6.6 gives the job `needs: [build, distro-build]` and downloads the SLC tarball, the UDF `.so` artifacts, the IT test binary and the two distribution-built artifacts. Task 6.4 states that same artifact set as what the composite action expects the calling job to have downloaded, so one bring-up sequence serves every live-database job. § Dependencies states that `distro-build` is now a prerequisite of `integration` and of `host-runtime`.
- **Promotes to ADR:** no

### [plan-review] The author-host and database-host records are keyed by family and version

- **Finding:** round 2 `[REQUIREMENT_CONFLICT]` BLOCKER. The author-host record was specified as one row per family, while the matrix carried seven images including two Ubuntu and two AlmaLinux references. Six rows cannot produce seven legs. With a bare family key, the harness argument of task 2.11, the cargo cache key of task 6.1 and `distro_build_matrix_matches_record` each broke, because a family alone cannot separate `ubuntu:22.04` from `ubuntu:24.04`.
- **Direction change:** the row key is the pair `<family> <version>` in both records, which preserves the version coverage the matrix already implied rather than collapsing it. The author-host record carries five fields, `<family> <version> <image-reference> <glibc-version> <build-path>`, stated in task 3.1 and in the `container/slc-platform-contract` scenario "Published platform support names one build path per author host". Task 2.11's harness takes `<family> <version>`, task 6.1's cache key is `<family>-<version>`, and both drift tests assert against rows. The database-host record takes the same key, which also fixes a version-specific prerequisite being recorded as a family-wide one. Decision [7] names the seven rows and their images, and the round-1 finding that stated four fields is marked superseded.
- **Promotes to ADR:** no

### [plan-review] The dual scope reads as settled fact in Design and Goals

- **Finding:** round 2 `[REQUIREMENT_CONFLICT]` BLOCKER. The plan published a scope statement its own decision log had already rejected, so a reader could implement the narrowed author-host-only claim.
- **Direction change:** § Design/Context and § Goals were re-read against decision [13]. Both already state the dual scope as decided fact, with no hedge and no open question, so no change was needed in either section. The remaining half of this finding is the plan's Status banner and `open-questions.md`, which the orchestrator owns and this pass did not touch.
- **Promotes to ADR:** no

### [plan-review] Advisory findings bundled with the round-2 blocker fixes

- **Finding:** round 2 raised eleven ADVISORY findings, including two the round-1 pass had deferred.
- **Direction change:** all eleven applied. Both deferred `[COMPLETENESS_GAP]` findings are now closed rather than deferred a third time. The read-only chroot self-test became task 1.9 and a CHANGED block on the `slim-image` scenario "Staged tree passes an in-build chroot self-test", which states that the run proves the tree starts without write access and MUST NOT be published as evidence that the mount set is complete, with § Residual risk stating that no live-database leg mounts the SLC read-only. The nested-path ordering became a parent rule in task 1.1, a new *AND* clause on "Staging creates the sandbox directory skeleton", a plain `mkdir` instruction in task 1.3 and an orphan-parent fixture in task 1.2. The rest: tasks 6.5 and 6.6 add `actions/checkout` before the composite action, tasks 6.1 and 6.6 state that each matrix is a literal list kept true by its drift test rather than a `fromJSON` generator, § Dependencies states the free disk, memory and passwordless `sudo` a `host-runtime` runner MUST provide, `slc-builder.txt` gains a third line holding the exact toolchain patch version that the `Dockerfile` builder `FROM` now pins and that tasks 2.11 and 3.1 name, decision [11] names group F as the CI owner, tasks 1.4, 2.3 and 2.9 name their tests, § Patterns states the one command that recovers the baked fingerprint from a binary for tasks 1.5, 1.6 and 1.7, and the prose defects are corrected (six records in § Summary, three semicolons replaced, the "Database host kernel" paragraph split).
- **Promotes to ADR:** no
