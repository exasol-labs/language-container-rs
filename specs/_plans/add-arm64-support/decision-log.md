# Decision Log: add-arm64-support

## Interview

**Q:** How should this single plan be scoped?
**A:** The full arm64 track (§0–§7) of GitHub issue #78.

**Q:** (verbatim) "we will start the implementation from the current state of PR #51, so any adjustments that it requires to land must be part of the plan."
**A:** PR #51's §0 landing adjustments (version bump + `Cargo.lock`, Dockerfile fail-fast guards for empty `TRIPLET`/`LOADER`) are in scope as tasks. (The original answer also listed "keeping the branch current with main" and "narrowing the PR description"; decision [10] later superseded and dropped both — a fresh in-repo branch off `main` is current by construction, and a superseding in-repo PR is not PR #51. See the [plan-review] "P0 execution model was undefined" entry.)

**Q:** How should §3 (per-arch release), §4 (arm64 CI leg), §6 (docs), §7 (targets json) be treated?
**A:** Tasks-only, no spec deltas — build/CI/release/docs mechanics live in `CLAUDE.md`, not the spec library. Author spec deltas only for the behavioral parts: §1 (cargo-exaudf build target), §2 (license-bundle arch coverage as a compliance requirement), §5 (Personal install path), and §0's arch-agnostic SLC-build capability if it is a genuine business requirement rather than pure Dockerfile mechanics.

**Q:** How should the full track be structured?
**A:** Phased and independently-landable — one plan.md organized P0→P1→P2→P3+P4→P5→P6 (P7 anytime), each phase a coherent, independently-verifiable increment on the PR #51 branch. Each landed phase bumps `[workspace.package].version` + the pinned `exasol-udf-sdk` dep + regenerated `Cargo.lock`.

## Design Decisions

### [1] §0 arch-agnostic build is captured as a slim-image spec delta

- **Decision:** Author a CHANGED delta to `container/slim-image` (generalize the builder's glibc-runtime collection to derive the multiarch triplet and PT_INTERP loader path; add a native-host-arch build scenario), rather than treating §0 as pure Dockerfile mechanics with no delta.
- **Alternatives:** No delta (mechanics-only, per `CLAUDE.md`).
- **Rationale:** Which architectures the container supports is a business capability tied to the DBA/platform persona (deploying on ARM). The loader-must-land-at-PT_INTERP requirement is a correctness invariant (else every UDF crashes), not a build mechanic — it belongs in the spec so it is not silently regressed.
- **Promotes to ADR:** no

### [2] License `targets` list the glibc triples for both arches, not only aarch64 musl

- **Decision:** Set `about.toml`'s `targets` to `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`.
- **Alternatives:** Add only `aarch64-unknown-linux-musl` (the issue's literal ask).
- **Rationale:** The shipped `exaudfclient` is built glibc (`rust:1.94-bookworm`, no `--target`), so the prior musl-only pin already under-reports gnu-gated dependencies. cargo-about unions the listed targets, so listing all four fixes the latent gnu gap and adds aarch64 in one move; over-attribution is compliance-safe, omission is not.
- **Promotes to ADR:** yes

### [3] New feature `container/crate-license-notices` rather than extending `os-license-notices`

- **Decision:** Give the Rust-dependency bundle (`about.toml` → `THIRD-PARTY-LICENSES.md`) its own feature.
- **Alternatives:** Add a per-arch scenario to `container/os-license-notices`.
- **Rationale:** `os-license-notices` is scoped to the OS-layer bundle (`about-os.toml`, apk + glibc/GCC runtime). Folding the Rust-crate concern in would blur its one-sentence responsibility. No existing feature covers the Rust-crate bundle, so a sibling feature is the clean home.
- **Promotes to ADR:** no

### [4] Remove the vestigial `targets/*.json` and the `COPY targets/` line (§7)

- **Decision:** Delete both `targets/*-dylib.json` files and the Dockerfile `COPY targets/ ./targets/` line.
- **Alternatives:** Keep them as a documented manual `-Zbuild-std` path.
- **Rationale:** Nothing in the repo feeds the JSON path to rustc — the CLI and the Dockerfile both build with rustup triple strings; the sole reference is the `Dockerfile.alpine` `COPY targets/` line. The `-Zbuild-std` comment at `crates/exasol-udf-sdk/build.rs:8` does not name the JSON, so it needs no cleanup. It is dead config; keeping it invites the assumption that it is load-bearing. (The JSON's `data-layout` is not stale — it is byte-for-byte identical to `rustc 1.94.1 --print target-spec-json`; the removal rests solely on the absence of a rustc consumer, not on staleness.)
- **Promotes to ADR:** yes

### [5] x86_64 keeps the unsuffixed release tarball name; aarch64 is suffixed

- **Decision:** The x86_64 build keeps `lc-rust-<version>.tar.gz`; add `lc-rust-<version>-aarch64.tar.gz` for arm64. The existing `files: lc-rust-*.tar.gz` release glob matches both.
- **Alternatives:** Suffix both (`-x86_64` / `-aarch64`).
- **Rationale:** `docs/installation.md` and the manual-install `curl` URL embed the unsuffixed name; suffixing x86_64 would break documented links for no benefit. Asymmetry is a small price for a non-breaking release.
- **Promotes to ADR:** yes

### [6] IT stays x86_64; arm64 end-to-end is manual

- **Decision:** Add an arm64 CI leg for workspace build + unit tests only; the integration suite stays x86_64.
- **Alternatives:** An arm64 integration leg via QEMU.
- **Rationale:** `exasol/docker-db` publishes amd64-only images; QEMU-emulating the privileged multi-GB DB is a non-starter. arm64 end-to-end verification stays manual against Exasol Personal until an arm64 DB image exists.
- **Promotes to ADR:** yes

### [7] Personal deployment is a new script/feature, not an `install.sh` flag

- **Decision:** Add `scripts/install-personal.sh` and a `container/personal-install` feature.
- **Alternatives:** A `--personal` mode on `scripts/install.sh`.
- **Rationale:** Personal exposes no BucketFS HTTP endpoint, so the transport (SSH + filesystem reconciliation) and the registration scope (`ALTER SYSTEM`, preserve existing entries) differ fundamentally from the upload+`ALTER SESSION` path. A separate path keeps each script's responsibility crisp rather than branching one script on two incompatible transports.
- **Promotes to ADR:** yes

### [8] Preserved platform knowledge (do not regress)

- **Decision:** Carry these field-verified facts into the specs and task notes so implementers cannot silently regress them: (a) the staged glibc loader MUST land at the binary's exact PT_INTERP path — Alpine's `/lib` is a real directory, not a symlink to `/usr/lib`, so a loader under `/usr/lib` alone leaves PT_INTERP dangling and every UDF dies as a bare `22002 VM crashed`; (b) the shipped `exaudfclient` is glibc, not musl; (c) the `SCRIPT_LANGUAGES` `#` fragment MUST point at the `exaudfclient` executable with no leading slash; (d) Personal's SSH port changes on every `exasol start` and MUST be read fresh, never cached; (e) IT stays x86_64 because `exasol/docker-db` is amd64-only.
- **Alternatives:** Leave the knowledge in the issue/PR history only.
- **Rationale:** Each fact took a field debugging session to establish; each maps to a `22002`/crash failure mode that is opaque to rediscover. Encoding them as normative spec clauses and task notes is the cheapest insurance against regression.
- **Promotes to ADR:** yes

### [9] cargo-exaudf `--target` is native-only; cross-compilation is out of scope

- **Decision:** Default the build triple to the host arch; accept `--target <triple>` for native builds on a matching-architecture host; do not provision a cross toolchain.
- **Alternatives:** Full cross-compilation support.
- **Rationale:** `.cargo/config.toml` maps musl targets to the host `musl-gcc`, so a cross build is doomed without a cross toolchain the project does not ship. Native-only matches the field-verified path and keeps the CLI simple.
- **Promotes to ADR:** no

### [10] Execution model: fresh in-repo branch, apply PR #51's diff as the first task

- **Decision:** Implement on a fresh in-repo branch off `main`; apply PR #51's arch-neutral four-file change (`Dockerfile.alpine`, `rust-toolchain.toml`, `.cargo/config.toml`, `targets/aarch64-unknown-linux-musl-dylib.json`) as P0 task 0.1. The superseding PR is this in-repo branch, not PR #51.
- **Alternatives:** Work on PR #51's fork branch and keep a "narrow the PR #51 description" task — rejected: the fork is cross-repository and unfetchable from `origin`, the implementer cannot commit to another user's fork, and editing its description needs write access this workflow does not have.
- **Rationale:** This supersedes the earlier decision **PR #51 branch currency**, which assumed the fork branch was the implementation base. `main` carries the x86_64-hardcoded `Dockerfile.alpine` (`x86_64-linux-gnu`, `ld-linux-x86-64.so.2`), an x86_64-only `rust-toolchain.toml` and `.cargo/config.toml`, and only the x86_64 `targets/*.json` (confirmed 2026-08-10). The arch-neutral change is therefore not inherited by an in-repo branch and must be applied explicitly as task 0.1. Branch currency is now moot — a fresh branch off `main` is current by construction — so the conditional-rebase task is dropped, and the "narrow the PR #51 description" task is dropped because a superseding in-repo PR is not PR #51.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] P0 execution model was undefined (BLOCKER)

- **Finding:** P0 assumed PR #51's four-file arch-neutral change was already present on the branch, but PR #51 is a cross-repository fork (`realtdegen`), unfetchable from `origin`, and `main` still carries the x86_64-hardcoded four files — so P0 had no way to obtain the arch-neutral build.
- **Direction change:** Chose the fresh-in-repo-branch model. Added task 0.1 to apply PR #51's four-file diff onto a branch off `main`; dropped the conditional branch-currency task and the "narrow the PR #51 description" task; rewrote decision [10]; and swept plan.md (Summary, Architecture diagram, Decision, Dependencies, § P0) for the "PR #51 is the base" assumption.
- **Promotes to ADR:** no

### [plan-review] P4 ordering vs #67–#70 was unspecified

- **Finding:** Task 4.3 said "coordinate" with the in-flight CI restructuring without setting an order; #67/#69 restructure the very jobs P3/P4 attach to.
- **Direction change:** 4.3 now fixes the order — this plan's P3 matrix and P4 arm leg land first against the current job graph (keeping the phase independently-landable); #67 and #69 rebase onto the arm matrix during their own work.
- **Promotes to ADR:** no

### [plan-review] Version-bump collisions under parallel Group A

- **Finding:** Group A schedules five phases in parallel, each bumping the shared version / `exasol-udf-sdk` / `Cargo.lock` triple, risking collisions and duplicate next-versions.
- **Direction change:** § Cross-cutting release hygiene now states bumps serialize on landing — each phase re-bumps from the then-current version at merge/rebase, so parallel development is fine and the bump resolves last, per landed phase.
- **Promotes to ADR:** no

### [plan-review] slim-image fail-fast MUST clause was unverified

- **Finding:** The scenario's "empty derived triplet or loader path MUST fail the build with a named error" clause had no Scenario Coverage row and no mapped test.
- **Direction change:** Added a Code review + Manual coverage row referencing task 0.3, plus a Manual Testing entry that forces an empty triplet and asserts the named guard error.
- **Promotes to ADR:** no

### [plan-review] cargo-exaudf integration coverage was overstated

- **Finding:** The three cargo-exaudf integration tests are `#[ignore]`d and plain `cargo test` skips them, so the coverage table overstated automated regression protection (only `host_triple_maps_arch` runs automatically).
- **Direction change:** Annotated the three integration rows as `#[ignore]`d (run with `--ignored`) and added a Checklist step `cargo test -p cargo-exasol-udf -- --ignored` on a musl-capable host.
- **Promotes to ADR:** no

### [plan-review] personal-install restart-cycle sub-clauses were unverified

- **Finding:** The "read fresh" and "preserves existing entries" rows were Unit-only, but each scenario carries a live `exasol stop`/`start` restart-cycle sub-clause the sourced-function test cannot exercise.
- **Direction change:** Changed both rows to Test Type = Unit + Manual and added a restart-idempotency re-run to the Manual Testing table.
- **Promotes to ADR:** no

### [plan-review] P5 task 5.1 bundled four independently-failing concerns

- **Finding:** Task 5.1 bundled tarball build, connection-detail reading, scp/extract reconciliation, and registration into one `[expert]` task; only two of the four had unit tests.
- **Direction change:** Split P5 into separately-verifiable tasks — 5.2 connection-detail reading + registration-string assembly + `ALTER SYSTEM` (unit-testable), and 5.3 transport + extraction + filesystem reconciliation (manual/live).
- **Promotes to ADR:** no

### [plan-review] SCRIPT_LANGUAGES #-fragment invariant duplicated across scripts

- **Finding:** The executable-path / no-leading-slash `#`-fragment rule — a `22002`-crash-if-wrong invariant — is hardcoded inline at `scripts/install.sh:123` and would be re-implemented independently in `install-personal.sh`, with nothing enforcing agreement.
- **Direction change:** Added task 5.1 to extract the fragment/registration assembly into a shared sourced helper `scripts/lib/script_languages.sh` that both scripts source; the 5.4 unit tests point at the helper, giving the invariant a single owner.
- **Promotes to ADR:** no

### [plan-review] False "stale data-layout" claim in ADR-bound decision [4]

- **Finding:** Decision [4] (promotes to ADR) justified removing `targets/*.json` partly on a "stale data-layout," but the JSON's `data-layout` is byte-for-byte identical to `rustc 1.94.1 --print target-spec-json` — the claim is false and would be enshrined permanently.
- **Direction change:** Removed the stale-data-layout justification from decision [4], plan.md § Dead Code Removal, and § Consequences; rested the §7 removal solely on "no in-repo rustc consumer; sole reference is the `Dockerfile.alpine` `COPY targets/` line," and noted the `build.rs:8` `-Zbuild-std` comment needs no cleanup.
- **Promotes to ADR:** no

### [plan-review] P3 task 3.1 broke the x86_64 integration leg (round 2, ADVISORY)

- **Finding:** Task 3.1 renamed `build-slc`'s single fixed-name `lc-tarball` artifact per arch, but two jobs download it by that exact name — `integration` (`ci.yml:354`) and `release` (`ci.yml:534`). Only the `release` consumer (task 3.2) was handled; nothing updated the `integration` download, so the per-arch rename would red the x86_64 IT leg that decision [6] requires to keep passing. Verified against `ci.yml`: upload at 288–292, integration download at 354–357, release download at 534–538.
- **Direction change:** Task 3.1 now states that the matrix rename MUST also update the `integration` job's download step (~`ci.yml:354`) to fetch the x86_64-named artifact, and names the `integration` job (not only `release`) as an affected consumer of the renamed `lc-tarball`.
- **Promotes to ADR:** no

### [plan-review] P5 task 5.1 over-claimed existing-entry preservation as extractable (round 2, ADVISORY)

- **Finding:** Task 5.1 listed "existing-entry preservation" among the logic extracted from `scripts/install.sh:123` into the shared helper. Verified against `install.sh`: line 123 assigns a single `RUST=…` value wholesale (no read/append) and line 128 uses scope-parameterized `ALTER ${SCOPE_UPPER}` (default `SESSION`) — there is no preservation logic to extract; preservation is net-new behavior required only on `install-personal.sh`'s `ALTER SYSTEM` path. Building preservation into the shared helper risked silently changing the shipped x86_64 path from overwrite to merge.
- **Direction change:** Scoped task 5.1's shared helper to `#`-fragment / registration-string assembly only (executable path, no leading slash); stated the `install.sh` refactor MUST preserve its current single-value `ALTER ${SCOPE}` overwrite behavior; and moved existing-entry preservation to task 5.2 as NEW logic unique to `install-personal.sh`'s `ALTER SYSTEM` path, not part of the shared helper. The single-owner design for the fragment invariant (round-1 finding) is unchanged.
- **Promotes to ADR:** no

### [plan-review] Interview answer contradicted decision [10] (round 2, ADVISORY)

- **Finding:** The § Interview line-9 answer still recorded "keeping the branch current with main" and "narrowing the PR description" as in-scope tasks, contradicting decision [10], which dropped both. An implementer skimming the Interview could reintroduce a dropped task.
- **Direction change:** Struck the two superseded clauses from the Interview answer and annotated that decision [10] superseded and dropped both, cross-referencing the P0-execution-model finding.
- **Promotes to ADR:** no

### [plan-review] crate-license-notices comment clause had no mapped verification (round 2, ADVISORY)

- **Finding:** The "Target set reflects the shipped glibc binary" scenario's third normative clause — "a comment in `about.toml` MUST record why the shipped binary is glibc and why both libc/arch triples are listed" — had no mapped verification; the Scenario Coverage row mapped only `about_toml_lists_gnu_triples`, which asserts triple presence, not the comment.
- **Direction change:** Added a sibling unit assertion `about_toml_comments_glibc_rationale` to the Scenario Coverage row to check the required `about.toml` comment is present, and extended task 2.2 to write that assertion (the comment itself is written by task 2.1).
- **Promotes to ADR:** no
