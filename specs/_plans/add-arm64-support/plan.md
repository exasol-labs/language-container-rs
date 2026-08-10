# Plan: add-arm64-support

## Summary

Deliver full aarch64/arm64 support for the Rust SLC and an Exasol Personal (local, Apple Silicon) install path, applying PR #51's arch-neutral four-file change onto a fresh in-repo branch off `main`. The work ships as seven independently-landable phases (P0 land the foundation → P1 tooling → P2 license bundle → P3+P4 release+CI → P5 Personal install → P6 docs; P7 dead-config removal anytime).

## Design

### Context

The verified baseline (field-tested on Exasol Personal, Apple Silicon, DB 2026.2.0-nano): once platform blockers are worked around, Rust UDFs run on aarch64 with no change to the SDK, runtime, or protocol. The remaining gaps are hardcoded x86_64 assumptions in the container build, the developer CLI, the license bundle, the release pipeline, CI, and the install scripts — plus a missing deploy path for Personal, which exposes no BucketFS HTTP endpoint.

- **Goals** — A native `docker build` on either architecture produces the matching SLC; `cargo exasol-udf build` targets the host arch; the shipped license bundle attributes every shipped architecture; releases publish a per-arch tarball; CI builds and unit-tests on arm64; an operator can install the SLC onto Exasol Personal; hard-won platform knowledge is preserved so implementers do not regress it.
- **Non-Goals** — Cross-compilation from the developer CLI (native only); an arm64 integration-test leg (the `exasol/docker-db` image is amd64-only, and QEMU-emulating the privileged multi-GB DB is a non-starter — arm64 end-to-end stays manual against Personal); changes to the SDK, runtime, or wire protocol (the baseline proves none are needed).

### Decision

Parameterize the existing single-arch pipeline by architecture rather than fork per-arch paths. The container derives its architecture from the build host (via PR #51's arch-neutral change, applied in P0); the CLI derives its target triple from `std::env::consts::ARCH`; the release and CI matrices add a native arm64 runner; the license bundle lists both architectures under cargo-about's union semantics. Personal deployment is a distinct install path because its constraint (no BucketFS endpoint) is architectural, not a flag on the existing script.

#### Architecture

```
   P0: fresh in-repo branch off main + apply PR #51's arch-neutral 4-file change
       (Dockerfile.alpine, rust-toolchain.toml, .cargo/config.toml, targets json)
                                     │  P0
        ┌────────────────┬──────────┼───────────────┬──────────────┐
        ▼                ▼          ▼                ▼              ▼
   P1 cargo CLI     P2 license   P4 arm64 CI    P5 Personal    P7 remove
   host-arch target  bundle both  build+unit     install path   vestigial
   + --target        arches       leg            (SSH + fs BFS)  targets/*.json
        │                │                            │
        └──────┐    ┌────┘                            │
               ▼    ▼                                 │
          P3 per-arch release asset                   │
               │                                       │
               └───────────────┬───────────────────────┘
                               ▼
                        P6 docs (matrix, 22002 note, escape hatch)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Host-architecture derivation | Dockerfile (`gcc -print-multiarch`, `readelf -l` PT_INTERP), CLI (`std::env::consts::ARCH`) | One code path serves both arches; no per-arch branch to drift |
| Union license evaluation | `about.toml` `targets` | Listing every shipped arch over-attributes safely; omission silently drops deps |
| Filesystem BucketFS reconciliation | `scripts/install-personal.sh` | Personal exposes no upload endpoint; the engine reconciles a bucket from the VM filesystem |
| Read-fresh, never cache | Personal SSH port from `deployment.json` | The port changes on every `exasol start` |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| §0 arch-agnostic build gets a `slim-image` spec delta | Treat as pure Dockerfile mechanics (no delta) | Which architectures the container supports is a business capability (mission persona: DBA deploying on ARM); the PT_INTERP loader-placement requirement is a correctness invariant, not a mechanic |
| License `targets` = glibc + musl for both arches | Add only `aarch64-unknown-linux-musl` per the issue | The shipped `exaudfclient` is glibc (built with no `--target`); the musl-only pin already under-reports gnu-gated deps — fix both at once (union makes over-listing safe) |
| x86_64 keeps unsuffixed tarball name; aarch64 gets `-aarch64` suffix | Suffix both (`-x86_64`/`-aarch64`) | `docs/installation.md` and the manual-install URL embed the unsuffixed name; non-breaking wins |
| Remove `targets/*.json` + `COPY targets/` (P7) | Keep as a documented `-Zbuild-std` path | Nothing feeds the JSON to rustc; the sole reference is `Dockerfile.alpine`'s `COPY targets/` line; it is dead config |
| IT stays x86_64 | arm64 IT leg via QEMU | `exasol/docker-db` is amd64-only; QEMU of the privileged DB is unworkable |
| Personal install is a new script/feature, not an `install.sh` flag | Add a `--personal` mode to `install.sh` | The transport (SSH + filesystem) and registration scope differ fundamentally; a separate path keeps each script's responsibility crisp |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| tools/cargo-exaudf | CHANGED | `tools/cargo-exaudf/spec.md` |
| container/slim-image | CHANGED | `container/slim-image/spec.md` |
| container/crate-license-notices | NEW | `container/crate-license-notices/spec.md` |
| container/personal-install | NEW | `container/personal-install/spec.md` |

Sections §3 (per-arch release), §4 (arm64 CI leg), §6 (docs), §7 (targets json) are build/CI/release/docs mechanics — implemented as tasks below, with no spec delta, per `CLAUDE.md`'s mechanics-out-of-specs rule.

## Impact

- **UDF authors** — `cargo exasol-udf build` builds for the host arch automatically; a new `--target <triple>` allows a native aarch64 build. No breaking change: on x86_64 the default is unchanged.
- **Operators** — a new `lc-rust-<version>-aarch64.tar.gz` release asset and a `scripts/install-personal.sh` path make ARM hosts and Exasol Personal first-class. The x86_64 asset name is unchanged, so existing install docs and links keep working.
- **License compliance** — the shipped `THIRD-PARTY-LICENSES.md` gains gnu-triple and aarch64 coverage; over-attribution only, no runtime effect.
- **No change** to the SDK, runtime, wire protocol, or the x86_64 integration suite.

## Cross-cutting release hygiene

Every phase that lands as its own change MUST bump `[workspace.package].version` in `Cargo.toml`, keep the pinned `exasol-udf-sdk` dependency in sync, and commit the regenerated `Cargo.lock` in the same change (per `CLAUDE.md`). Stated once here; each phase carries a single bump task rather than repeating it per task.

Version bumps serialize on landing. Group A phases develop in parallel, but the shared version / `exasol-udf-sdk` / `Cargo.lock` triple is not a merge-conflict risk because each phase re-bumps from the then-current `Cargo.toml` version at its merge or rebase. The bump — and the `Cargo.lock` regeneration — resolves last, per landed phase; parallel development is fine, and two phases never claim the same next version.

## Dependencies

- `ubuntu-24.04-arm` GitHub-hosted runners (free + GA for public repos since 2025-08-07).
- `exapump` linux-aarch64 asset (present from the CI-pinned v0.9.2 onward) — needed only if an arm64 path invokes exapump; the arm64 CI leg does not.
- `cargo-about` 0.9.0 (already installed in CI) — union target evaluation.
- PR #51's arch-neutral four-file diff (from the external fork `realtdegen:feat/aarch64-arm64-support`) as the source of the P0 change, applied onto a fresh in-repo branch off `main` (see decision [10] and § P0).

## Implementation Tasks

### P0 — Land the arch-agnostic SLC build (foundation)

**Branch model.** Implementation runs on a fresh in-repo branch off `main` (e.g. `feat/add-arm64-support`), NOT on PR #51. PR #51 lives on the external fork `realtdegen:feat/aarch64-arm64-support` — cross-repository and unfetchable from `origin` — and `main` still carries the x86_64-hardcoded `Dockerfile.alpine` (`x86_64-linux-gnu`, `ld-linux-x86-64.so.2`), an x86_64-only `rust-toolchain.toml` and `.cargo/config.toml`, and only the x86_64 `targets/*.json`. The arch-neutral four-file change is therefore NOT inherited — it is applied as the first task from PR #51's diff. The superseding PR is this in-repo branch, not PR #51 (see decision [10]).

- [ ] 0.1 Apply PR #51's arch-neutral four-file change onto the working branch: make `Dockerfile.alpine` derive the Debian multiarch triplet (`gcc -print-multiarch`) and the loader's `PT_INTERP` path (`readelf -l`) from the build host instead of hardcoding `x86_64-linux-gnu`/`ld-linux-x86-64.so.2`; make `rust-toolchain.toml`'s `targets` arch-neutral; add the aarch64 musl linker mapping to `.cargo/config.toml`; add `targets/aarch64-unknown-linux-musl-dylib.json`. Source the diff from the local `pr-51` ref or reconstruct the four files; verify against `main` that all four changes land. [expert]
- [ ] 0.2 Bump `[workspace.package].version` and the pinned `exasol-udf-sdk` dependency in `Cargo.toml`; regenerate and commit `Cargo.lock`.
- [ ] 0.3 Add fail-fast guards in `Dockerfile.alpine` so an empty derived `TRIPLET` or `LOADER` aborts the build with a named error; verify the loader is staged at the binary's exact `PT_INTERP` path (Alpine `/lib` is a real directory, not a symlink to `/usr/lib` — a loader staged elsewhere leaves `PT_INTERP` dangling and every UDF dies as a bare `22002 VM crashed`). [expert]

### P1 — cargo-exasol-udf: host-arch target + `--target` override

- [ ] 1.1 Add a pure `host_triple()` helper in `crates/cargo-exasol-udf/src/build.rs` deriving `<arch>-unknown-linux-musl` from `std::env::consts::ARCH`; add a `build_tests.rs` sibling wired via `#[cfg(test)] #[path = "build_tests.rs"] mod tests;` with unit tests for the `x86_64`/`aarch64` mappings.
- [ ] 1.2 Parse an optional `--target <triple>` flag from the args slice in `build::run` (manual parse, matching the crate's existing dispatch); default to `host_triple()` when absent. Remove the hardcoded `MUSL_TARGET` constant.
- [ ] 1.3 Thread the selected triple through the `cargo build --target` invocation, the printed `.so` output path, and `ensure_musl_target()` (install the selected triple when missing).
- [ ] 1.4 Update `crates/cargo-exasol-udf/tests/build.rs::build_produces_musl_so` to assert the host triple rather than a hardcoded `x86_64-unknown-linux-musl`; add an integration test for the `--target` override path.
- [ ] 1.5 Update `usage()` in `main.rs` to document `--target`; update the `x86_64-unknown-linux-musl` references in `README.md` (lines 61, 68), `docs/writing-a-udf.md` (11, 602, 614), and `docs/cargo-ecosystem.md` (84, 87) to reflect the host-arch default.
- [ ] 1.6 Version bump per the cross-cutting rule.

### P2 — License bundle: arch + libc coverage

- [ ] 2.1 Set `about.toml`'s `targets` to `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`; add a comment recording that the shipped `exaudfclient` is glibc and that both libc/arch triples are listed because cargo-about unions them (over-attribution is safe).
- [ ] 2.2 Add a test asserting `about.toml` lists every shipped architecture's triple (`about_toml_lists_gnu_triples`) and a sibling assertion (`about_toml_comments_glibc_rationale`) confirming the task-2.1 comment recording the glibc rationale and the four-triple listing is present; regenerate `THIRD-PARTY-LICENSES.md` via `dist/generate-licenses.sh` and confirm the union adds aarch64/gnu deps without dropping the prior x86_64 set.
- [ ] 2.3 Version bump per the cross-cutting rule.

### P3 — Release: per-arch SLC tarball asset

- [ ] 3.1 Make the `build-slc` job a matrix over runner architecture (`ubuntu-latest` + `ubuntu-24.04-arm`); upload the per-arch tarball as distinctly-named artifacts, replacing the current fixed-name `lc-tarball` upload (`.github/workflows/ci.yml:288`). Two downstream jobs download `lc-tarball` by that exact name and BOTH must be updated in the same change: the `integration` job's download step (`.github/workflows/ci.yml` ~354) MUST fetch the x86_64-named artifact so the x86_64 IT leg (decision [6]) keeps passing, and the `release` job (handled in task 3.2). Renaming the artifact without updating the `integration` consumer reds the x86_64 IT leg CI-only.
- [ ] 3.2 In the `release` job, collect both arch tarballs; keep the x86_64 build's historical unsuffixed name `lc-rust-<VER>.tar.gz` and add `lc-rust-<VER>-aarch64.tar.gz` (the existing `files: lc-rust-*.tar.gz` glob already matches both).
- [ ] 3.3 Update `docs/installation.md` (Step 1, ~lines 39–47) to list both assets and state that the unsuffixed name is the x86_64 build.
- [ ] 3.4 Version bump per the cross-cutting rule.

### P4 — CI: arm64 build + unit-test leg (IT stays x86_64)

- [ ] 4.1 Add an `ubuntu-24.04-arm` leg running `cargo build --workspace` + unit tests only — no coverage, Sonar, or IT duplication.
- [ ] 4.2 Add a `ci.yml` comment recording why IT stays x86_64: `exasol/docker-db` publishes amd64-only images and QEMU-emulating the privileged multi-GB DB is a non-starter; arm64 end-to-end stays manual against Personal.
- [ ] 4.3 Ordering vs #67–#70: land this plan's P3 matrix and P4 arm leg first, attaching them to the current `build-slc` / unit-test job graph — this keeps the phase independently-landable and unblocked by open PRs. #67 (unblock `build-slc`) and #69 (flatten the job graph) then rebase onto the arm matrix during their own work. Do NOT block P3/P4 on the not-yet-landed flattened graph.
- [ ] 4.4 Version bump per the cross-cutting rule.

### P5 — Exasol Personal install path

- [ ] 5.1 Extract the `SCRIPT_LANGUAGES` `#`-fragment / registration-string **assembly only** — the `RUST=…#buckets/…/exaudf/exaudfclient` value building (executable path, no leading slash) — out of the inline expression at `scripts/install.sh:123` into a shared sourced helper `scripts/lib/script_languages.sh`; refactor `install.sh` to source it. `install.sh:123` assigns a single `RUST=…` value wholesale (no read of the current parameter, no append) and its `ALTER ${SCOPE_UPPER}` statement (default `SESSION`) overwrites the parameter; the refactor MUST preserve that single-value overwrite behavior. Existing-entry preservation is NOT present in `install.sh` and is therefore NOT extracted here — it is new logic added in task 5.2 for `install-personal.sh`'s `ALTER SYSTEM` path. This gives the `22002`-crash-if-wrong fragment invariant (decision [8], fact c) a single owner so `install.sh` and `install-personal.sh` cannot drift apart. [expert]
- [ ] 5.2 Add `scripts/install-personal.sh` connection-detail reading and registration: read `connection.sshPort` and the key path from `~/.exasol/personal/deployments/<name>/deployment.json` on every run (never cache the port); assemble the registration string via `scripts/lib/script_languages.sh` (the shared assembly helper from 5.1); issue `ALTER SYSTEM SET SCRIPT_LANGUAGES` over `8563` with the `#` fragment pointing at the `exaudfclient` executable and no leading slash. Existing-entry preservation is NEW logic added here — read the current `SCRIPT_LANGUAGES` value and append the `RUST=…` entry rather than overwriting — unique to this `ALTER SYSTEM` path and not part of the shared helper. This is the unit-testable parse-and-assembly half. [expert]
- [ ] 5.3 Add the transport and filesystem-reconciliation half to `scripts/install-personal.sh`: build the aarch64 SLC (or accept `SLC_TARBALL`); scp the tarball over the SSH port read in 5.2 and extract it into `/var/lib/exa/bucketfs/<service>/<bucket>/<slc-name>/` on the VM; confirm the engine reconciles a real bucket, visible to UDFs at `/buckets/<service>/<bucket>/<slc-name>/`. This half is manual/live — no arm64 CI DB image exists. [expert]
- [ ] 5.4 Add `scripts/tests/install-personal-test.sh` (sourced-function assertions, architecture-independent) covering the executable-path fragment format and existing-entry preservation, pointed at `scripts/lib/script_languages.sh`; wire it into the unit-test CI leg.
- [ ] 5.5 Update `README.md` (Personal is listed first among instances) and `docs/installation.md` to add the Personal path so Personal users no longer dead-end at the BucketFS-upload step.
- [ ] 5.6 Version bump per the cross-cutting rule.

### P6 — Docs

- [ ] 6.1 Add a `22002` troubleshooting note to `docs/installation.md`: `22002 VM crashed` almost always means the engine could not execute the UDF client — check the `#` fragment points at the `exaudfclient` executable (not its directory) and has no leading slash.
- [ ] 6.2 Document the glibc-cdylib escape hatch in `docs/writing-a-udf.md`: plain `cargo build --release -p my-udf` (host glibc, no `--target`) loads fine because the container bundles the matching glibc runtime — it is how CI builds the IT fixtures. Present it as the arm64 workaround until P1 lands and as a general alternative.
- [ ] 6.3 Add a platform/arch support matrix (Docker-db x86_64 / SaaS / Personal aarch64 → which tarball, which install path, which UDF build target); until P1/P3/P5 land, state explicitly that the scripted path does not support Personal.
- [ ] 6.4 Version bump per the cross-cutting rule.

### P7 — Remove vestigial `targets/*.json` (anytime after P0)

- [ ] 7.1 Remove `targets/x86_64-unknown-linux-musl-dylib.json` and `targets/aarch64-unknown-linux-musl-dylib.json`, and the `COPY targets/ ./targets/` line in `Dockerfile.alpine`; confirm no build regresses (nothing passes the JSON path to rustc; the CLI and Dockerfile use rustup triple strings only).
- [ ] 7.2 Version bump per the cross-cutting rule.

## Parallelization

| Parallel Group | Phases |
|----------------|--------|
| Foundation | P0 |
| Group A (after P0) | P1, P2, P4, P5, P7 |
| Group B (after P0 + P2) | P3 |
| Group C (after P1 + P3 + P5) | P6 |

Sequential dependencies:
- P0 → everything (arch-agnostic build is the base).
- P2 → P3 (the release asset must ship a correct per-arch license bundle).
- P1, P3, P5 → P6 (docs describe the landed capabilities).

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Constant | `crates/cargo-exasol-udf/src/build.rs` `MUSL_TARGET` | Replaced by the derived `host_triple()` (P1) |
| Files | `targets/x86_64-unknown-linux-musl-dylib.json`, `targets/aarch64-unknown-linux-musl-dylib.json` | Vestigial; never fed to rustc; sole reference is the `COPY targets/` line (P7) |
| Dockerfile line | `Dockerfile.alpine` `COPY targets/ ./targets/` | Sole consumer of the removed JSONs (P7) |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| cargo-exaudf: build produces a fully-static musl .so | Unit + Integration (integration `#[ignore]`d; run with `--ignored`) | `crates/cargo-exasol-udf/src/build_tests.rs`; `crates/cargo-exasol-udf/tests/build.rs` | `host_triple_maps_arch` (auto); `build_produces_musl_so` (ignored) |
| cargo-exaudf: build installs the musl target when missing | Integration (`#[ignore]`d; run with `--ignored`) | `crates/cargo-exasol-udf/tests/build.rs` | `build_installs_missing_target` |
| cargo-exaudf: build honors an explicit target override | Integration (`#[ignore]`d; run with `--ignored`) | `crates/cargo-exasol-udf/tests/build.rs` | `build_honors_target_override` |
| slim-image: Builder toolchain and glibc runtime | Integration | `crates/it/tests/` (db-roundtrip, x86_64) | `db_roundtrip_alpine` |
| slim-image: SLC builds natively for the host architecture | Integration (x86_64) + Manual (aarch64) | `crates/it/tests/` db-roundtrip; manual field build | `db_roundtrip_alpine`; see Manual Testing |
| slim-image: empty derived triplet or loader path fails the build with a named error | Code review + Manual | `Dockerfile.alpine` fail-fast guard (task 0.3) | see Manual Testing |
| crate-license-notices: License bundle covers every shipped architecture | Unit | `dist/` config assertion (about.toml targets) | `about_toml_lists_all_shipped_targets` |
| crate-license-notices: Target set reflects the shipped glibc binary | Unit | `dist/` config assertion | `about_toml_lists_gnu_triples`; `about_toml_comments_glibc_rationale` (asserts the required `about.toml` comment recording why the shipped binary is glibc and why all four libc/arch triples are listed — the scenario's third normative clause) |
| crate-license-notices: Generated manifest ships in the tarball | Integration | `crates/it/tests/` tarball inspection | `tarball_carries_third_party_licenses` |
| personal-install: Connection details are read fresh on every run | Unit + Manual | `scripts/tests/install-personal-test.sh`; live restart-cycle re-run | `reads_ssh_port_from_deployment_json`; see Manual Testing |
| personal-install: SLC is deployed via filesystem BucketFS reconciliation | Manual | live Personal deployment | see Manual Testing |
| personal-install: Registration targets the exaudfclient executable | Unit | `scripts/tests/install-personal-test.sh` (sources `scripts/lib/script_languages.sh`) | `fragment_points_at_executable_no_leading_slash` |
| personal-install: Registration is system-scoped and preserves existing entries | Unit + Manual | `scripts/tests/install-personal-test.sh` (sources `scripts/lib/script_languages.sh`); live restart-cycle re-run | `preserves_existing_script_languages`; see Manual Testing |
| personal-install: A registered Rust UDF executes on Personal | Manual | live Personal deployment | see Manual Testing |

The aarch64 halves of the slim-image and personal-install scenarios are manual because no arm64 Exasol DB image exists for CI (see the IT-stays-x86_64 decision). Manual evidence is the field-test procedure below.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| cargo-exaudf | `cargo exasol-udf build` on an aarch64 host | prints `target/aarch64-unknown-linux-musl/release/lib<crate>.so` |
| cargo-exaudf | `cargo exasol-udf build --target aarch64-unknown-linux-musl` | builds for the given triple, prints its path |
| slim-image | `docker build -f Dockerfile.alpine --target artifact --output type=local,dest=/tmp/out .` on Apple Silicon; `file /tmp/out` extracted `exaudf/exaudfclient` | `ELF 64-bit … ARM aarch64 … interpreter /lib/ld-linux-aarch64.so.1` |
| slim-image (fail-fast) | `docker build` with the triplet derivation forced to emit an empty value (temporary edit to the derive step) | build aborts at the guard with the named error (e.g. `error: empty multiarch triplet`), not a downstream `cp` failure |
| crate-license-notices | `bash dist/generate-licenses.sh` then inspect `dist/THIRD-PARTY-LICENSES.md` | manifest present; contains attributions for the x86_64 set plus any aarch64/gnu-gated crates |
| personal-install | `scripts/install-personal.sh` against a running Personal deployment, then `scalar_double(21)` | returns `42`; `scalar_double(NULL)` returns `NULL`; resolves again after `exasol stop && exasol start` |
| personal-install (restart idempotency) | after a successful install, run `exasol stop && exasol start`, then re-run `scripts/install-personal.sh` | the re-run reads the reassigned SSH port fresh (not cached), preserves every pre-existing `SCRIPT_LANGUAGES` entry, and `scalar_double(21)` still returns `42` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Test (ignored, musl-capable host) | `cargo test -p cargo-exasol-udf -- --ignored` | 0 failures (builds a real musl `.so`; requires the musl toolchain — the cargo-exaudf integration tests are `#[ignore]`d and skipped by plain `cargo test`) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
| Integration (x86_64) | `cargo test -p it --features integration` | 0 failures |
| SLC build | `docker build -f Dockerfile.alpine --target artifact --output type=local,dest=/tmp/out .` | Exit 0; tarball produced |
