# Decision Log: change-slc-runtime-debian

## Interview

**Q:** Should this plan cover all of PR1 (sections 1–7 of issue #84), or split the `cargo exasol-udf validate` ELF checks (section 7) into a separate follow-up plan?
**A:** All of PR1 together, in this one plan.

**Q:** The new root Dockerfile replacing the deleted `Dockerfile.alpine` — what should it be named?
**A:** `Dockerfile` (unqualified — no Alpine/Debian disambiguation is needed once Alpine is fully gone).

**Q:** How should CI assert the bzip2 link mode is dynamic, so a future base-image refresh can't silently flip it to static?
**A:** Run `ldd` (or `readelf -d`) on the staged `exaudfclient` binary inside the built image/tarball in CI and fail if `libbz2.so.1` is not a `NEEDED` entry — verify the actual shipped artifact, not just the builder's installed packages.

**Q:** `specs/mission.md`'s Tech Stack row names a nonexistent `Dockerfile.debian` and is stale about Alpine. The issue says to route mission changes through `/speq:mission`, but `CLAUDE.md` treats `mission.md` as owned by that separate interview.
**A:** Have the planner edit `specs/mission.md`'s Tech Stack row directly as a task in this plan (bypass a separate `/speq:mission` run for this one line — just fix the "Container base" row to reflect the new single `Dockerfile` / `debian:trixie-slim` reality).

## Design Decisions

### [1] Replace the Alpine runtime with a curated `debian:trixie-slim` staged tree

- **Decision:** The SLC is built from a single root `Dockerfile` in three stages — a `rust:1.94-trixie` builder, a `debian:trixie-slim` donor/packager that stages a curated `/slc` tree, and a `FROM scratch` artifact stage. The staged tree contains only the glibc runtime, the documented UDF library surface, the client, `build_info/` and the notice bundles: no shell, package manager, coreutils, Rust toolchain or vendored registry.
- **Alternatives:** (a) Keep the Alpine envelope with glibc bundled inside it — rejected: it ships a musl userland the UDF never uses, wrapped around the glibc runtime the UDF entirely depends on, and forces the loader to be threaded through Alpine's non-usr-merged `/lib` by hand. (b) Flatten the whole `debian:trixie-slim` rootfs as the Alpine build flattened Alpine's — rejected: it ships a shell, `apt` and coreutils no UDF uses, keeps the shipped surface unreviewable, and drags GPL-2.0-only and BSD-2-Clause attribution obligations along with it.
- **Rationale:** The extracted tree is the UDF's root filesystem, so its contents are a deliberate product decision. Staging explicitly makes that decision reviewable, removes the two-libc split, lets the loader path fall out of the donor's own usr-merge layout, and shrinks both the artifact and the compliance surface.
- **Promotes to ADR:** yes
- **Supersedes:** "Alpine image — build the client binary for x86_64-unknown-linux-musl" (`alpine-image-musl-client-binary`, ADR 005). That decision's musl rationale was already abandoned in practice by the glibc-bundling pivot its own Consequences section records; this plan completes the pivot by removing the Alpine layer entirely.

### [2] Runtime locale stays `LANG=C.UTF-8`, re-homed to Debian with the locale data staged

- **Decision:** Keep `ENV LANG=C.UTF-8` on the staging stage and additionally stage `/usr/lib/locale/C.utf8` into the tree. No locale package is installed and no `locale-gen` runs.
- **Alternatives:** Install Debian's `locales` and generate `en_US.UTF-8` — rejected: it adds weight for no UDF-visible benefit, and `debian:trixie-slim` already ships the compiled `C.utf8` locale (verified).
- **Rationale:** `C.UTF-8` gives the UTF-8 string semantics UDF text handling needs. The reason for keeping it survives the base change unaltered; only the justification "musl has no `locales` package" no longer applies. The image-level `ENV` does not survive tarball extraction, so the staged locale data — not the `ENV` line — is what makes the locale resolvable inside the sandbox; that distinction was previously implicit and is now spelled out in the spec.
- **Promotes to ADR:** yes
- **Supersedes:** "Alpine runtime uses LANG=C.UTF-8 instead of locale-gen" (`alpine-runtime-lang-c-utf-8`, ADR 005).

### [3] Derive every architecture-dependent path from the donor; hand the derived values across stages in files

- **Decision:** The multiarch triplet and the client's `PT_INTERP` loader path are derived in the builder and written to `/slc-meta/` for the staging stage to read. The staged usr-merge layout is derived by reading which of `/lib`, `/lib64`, `/bin`, `/sbin` are symlinks in the donor and reproducing those with the donor's own targets. Symlink creation precedes file staging so all real files land under `/slc/usr`.
- **Alternatives:** (a) Re-derive both values in the donor — rejected: `debian:trixie-slim` has neither `binutils` nor `dpkg-architecture` (verified), so there is nothing there to derive with. (b) Hardcode the per-architecture symlink set — rejected: x86_64 needs `/lib64` for `PT_INTERP=/lib64/ld-linux-x86-64.so.2` and aarch64 has no `/lib64` at all, so a hardcoded list is an arch-specific defect waiting for the other runner.
- **Rationale:** A single Dockerfile must produce a correct tree on both runners with no cross-compilation. Ordering matters and is subtle: `/usr/lib64/ld-linux-x86-64.so.2` is itself a symlink into `/usr/lib/<triplet>/`, so `cp -L` of the `PT_INTERP` path only resolves correctly once `/slc/lib64 -> usr/lib64` exists over a real `/slc/usr/lib64` directory.
- **Promotes to ADR:** yes

### [4] Ship the "variant E" library surface and make vendoring the contract for everything else

- **Decision:** Stage OpenSSL 3 (with `ossl-modules` and `engines-3`), `zlib`, `bzip2` and `zstd` beyond the client's own `ldd` closure, alongside the glibc runtime, its compatibility stubs, and the dlopen-only NSS/resolver modules. Anything a UDF links dynamically outside that surface must be vendored into the `.so`, and `cargo exasol-udf validate` reports violations.
- **Alternatives:** (a) Stage only the client's `ldd` closure — rejected: a UDF using `native-tls` or a compression `-sys` crate would fail at `dlopen` with a raw loader error and no diagnosis path. (b) Stage the donor's full library surface — rejected: ~29 MB raw for a set nobody enumerated, versus 8.2 MB raw / 1.1 MB gzipped (measured on aarch64) for the curated set.
- **Rationale:** These four libraries are what real `-sys` crates reach for. Naming them makes the boundary a published contract rather than an accident of the client's own link closure, and pairs the contract with a build-time check so authors learn about a violation on their own machine instead of in a UDF failure.
- **Promotes to ADR:** yes

### [5] `exaudfclient` links no bzip2 at all; drop the `libbz2-dev` pin and its CI assertion

- **Decision:** Do not install `libbz2-dev` in the builder, and do not assert a bzip2 `DT_NEEDED` entry on the shipped `exaudf/exaudfclient` anywhere in CI. `libbz2.so.1` stays staged in `/slc` — that guarantee is for UDF authors' own crates that link `bzip2-sys` dynamically, not for the client binary.
- **Corrects:** the planning-time entry below (originally "[5] Pin bzip2 to dynamic linkage via `libbz2-dev`, and assert it on the shipped artifact"), whose premise — that `exaudfclient` links bzip2, dynamically or statically, and only the link *mode* was undetermined — is false. Verified via `readelf` on three independent builds (a fresh `rust:1.94-trixie` build, the previously shipped Alpine-based artifact, and a local host build): `exaudfclient`'s `DT_NEEDED` set and `.dynsym` carry zero bzip2 references in any of the three. Root cause, traced in the `exarrow-rs` 0.13.0 source: its only use of the `bzip2` crate is inside `src/import/csv.rs` and `src/export/csv.rs`, gating CSV `IMPORT`/`EXPORT` local-file compression — a code path this project's `ExaConnection` usage (`query`/`query_for_each`/`execute` only) never reaches. Rust's whole-program dead-code elimination correctly drops the entire unreachable `bzip2`/`bzip2-sys` unit at link time, in both debug and release profiles, on both architectures. The `#82` spike's "needed by exarrow-rs connect-back path" note was a mistaken inference from the dependency graph's existence, not from an observed reachable call.
- **Alternatives:** (a) Keep `libbz2-dev` as a "dormant" pin against a future reachable code path — rejected: it currently does nothing (the probe's outcome is unobservable in the artifact either way), and a pin that pins nothing is worse than no pin, since it invites the same false "this is exercised" reading that caused the original mistake. (b) Force a reachable bzip2 call (e.g., a fixture UDF that exercises `exarrow-rs`'s CSV compression path) so the original assertion becomes true — rejected: pure scope creep to satisfy a test, not a real UDF need, and out of this plan's Non-Goals.
- **Rationale:** A CI assertion should verify a real property of the shipped artifact. Asserting a `DT_NEEDED` entry that can never appear (given the current, unchanged `ExaConnection` surface) would either always fail — blocking every build — or force manufacturing a fake call site purely to satisfy the check. Neither serves authors. The compression-library staging story (variant E) is unaffected: `libbz2.so.1` is staged because real UDF `-sys` crates reach for it, independent of whether the SLC's own client does.
- **Promotes to ADR:** yes (supersedes the original entry's ADR-bound rationale before it is ever promoted)

### [6] The glibc floor is 2.41, measured on this plan's own image pair, and lives in one committed file

- **Decision:** Record the floor as `2.41` in `crates/cargo-exasol-udf/slc-glibc-floor.txt`, read it from the CLI via `include_str!`, and have the tarball contract test assert that the committed value equals the highest `GLIBC_x.y` version the staged `libc.so.6` defines and that the shipped client references nothing above it.
- **Alternatives:** (a) Inherit the issue's "2.41" figure without re-deriving it — rejected: that number was measured under the variant D / Rust 1.97 combination, and this plan pins the builder to `rust:1.94-trixie`, so the figure had to be established for the actual image pair before being published to authors. (b) A `const` in the CLI source — rejected: the pipeline would then have to grep Rust source to check for drift. (c) Read the floor from the tarball at author time — rejected: authors do not have the tarball.
- **Rationale:** The open question flagged in the planning brief is now resolved by measurement rather than assumption: `debian:trixie-slim` ships Debian GLIBC `2.41-12+deb13u3`, so the floor is 2.41, and it is a property of the runtime distro rather than of the Rust toolchain — the toolchain version governs what the *client* references, not what the container *provides*, and both stages are Debian 13. Recording it once and having the container build verify it against the shipped `libc.so.6` means the number authors read can never drift from what ships, so the PR2 toolchain bump cannot silently invalidate it either.
- **Promotes to ADR:** yes

### [7] `validate` errors above the floor, warns on unknown dependencies, and reads the ELF once

- **Decision:** `validate` performs one ELF read yielding the entry symbols, the `DT_NEEDED` sonames and the highest `GLIBC_x.y` reference. An artifact above the floor is a hard error; a `DT_NEEDED` entry outside the SLC surface is a warning that `--deny-unknown-deps` escalates to an error. The `nm` shell-out is removed in the same change. Checks run ELF read → entry symbols → glibc floor → `DT_NEEDED` → dlopen ABI/fingerprint.
- **Alternatives:** (a) Warn on both — rejected for the floor: such an artifact cannot load, and raising the floor from ~2.36 to 2.41 means no artifact that validated before starts failing. (b) Error on both — rejected for dependencies: an existing artifact may link an unstaged library and still work on today's full Alpine userland, so failing it on day one blocks authors before they can add a vendoring feature. (c) Keep `nm` for symbols and parse only for the new checks — rejected: two mechanisms reading the same dynamic section put ELF knowledge in two places, and dropping the shell-out removes the binutils requirement from the author's host as a side effect.
- **Rationale:** The severity of each check follows whether the artifact can load at all. Running the platform checks before `dlopen` also makes them testable with a generated fixture whose stub `libc.so.6` names a version above the floor — the failure is reached without ever loading the library. `goblin` was chosen over `object` because it surfaces both `DT_NEEDED` and the `.gnu.version_r` table directly, and it is MIT, already on `deny.toml`'s allow list.
- **Promotes to ADR:** yes

### [8] Amend ADR 024 with a pointer; reword only ADR 025's live platform clause

- **Decision:** Add one pointer line to ADR 024 noting that the bundled glibc runtime moved to Debian 13 while the glibc-dynamic-cdylib artifact model itself did not change, and leave its Context, Decision, Options and Consequences untouched. Reword ADR 025's PT_INTERP/usr-merge parenthetical so the platform fact it exists to preserve matches the staged tree, leaving its "integration tests stay x86_64-only" clause alone. Change no other ADR body text.
- **Alternatives:** Rewrite ADR 024's body as the issue's disposition table suggested — rejected: nothing in that body is Alpine-specific or falsified by this change (musl still cannot produce a cdylib; the SLC still bundles the matching glibc runtime), and editing it would contradict the same brief's rule that supersession is a status/pointer change rather than a rewrite of what was decided at the time.
- **Rationale:** The two ADRs need different treatment because the text at issue plays different roles. ADR 024's body is historical rationale that remains true, so a pointer suffices. ADR 025's clause is a *normative platform fact the ADR was written to carry forward* — its whole purpose is to stay accurate, so leaving it asserting that `/lib` is a real directory would preserve a falsehood as a MUST.
- **Promotes to ADR:** no

### [9] One tarball contract test owns every shipped-artifact assertion

- **Decision:** All structural assertions about the shipped artifact live in a single `dist/tests/slc_tarball_test.sh <tarball>`, run on both `build-slc` matrix legs and by `scripts/ci-it-local.sh`. The in-build `chroot` self-test stays in the Dockerfile.
- **Alternatives:** (a) Scatter the assertions across the Dockerfile, the CI YAML and the Rust integration suite — rejected: the ELF assertions need `readelf`, which the donor image lacks, and CI-YAML assertions do not run locally. (b) Move the `chroot` self-test into the shell test — rejected: `chroot` needs root, which the Docker build has by construction and a local test run does not; the alternative would be a silent skip, and the shell test asserts the equivalent structural property (the `PT_INTERP` path resolves inside the tree) without root.
- **Rationale:** One owner for the artifact contract keeps the assertions in a place that runs identically in CI and locally, and gives the aarch64 leg — which can never run integration tests — real structural coverage. The Dockerfile keeps only the assertion that must run as root.
- **Promotes to ADR:** no

### [10] Collapse the duplicate "slim and self-sufficient" scenario pair rather than rewriting both

- **Decision:** `container/slim-image` keeps one "Runtime stage is slim and self-sufficient" scenario and drops the near-duplicate "Alpine runtime stage is slim and self-sufficient". Two scenarios asserting a runnable tagged image ("docker build produces a tagged slim image", "Binary runs and reports its usage in the image") are replaced by scenarios about the artifact tarball and the in-build `chroot` self-test.
- **Alternatives:** Reword all four in place — rejected: the duplicate pair had already drifted apart (one required `libzmq` via `apk`, which has been stale since zmq became statically linked), and the two image-premised scenarios were already false, since the Dockerfile's last stage is `artifact` and a plain `docker build -t …` has not produced an image with `/exaudf/exaudfclient` for some time.
- **Rationale:** Both defects came from scenarios describing a build shape that no longer existed. Re-anchoring them on the tarball — the thing that actually ships and the thing the contract test can inspect — removes the drift at its source.
- **Promotes to ADR:** no

### [11] Version bump to 0.23.0

- **Decision:** Bump `[workspace.package].version` from `0.22.1` to `0.23.0`, tracking it in the pinned `exasol-udf-sdk` `[workspace.dependencies]` entry.
- **Alternatives:** A patch bump — rejected: `validate` gains new subcommand behaviour and a new flag, which is a minor-level addition.
- **Rationale:** No published API is removed or altered incompatibly, so minor is correct under SemVer. The bump changes `EXA_SDK_FINGERPRINT`, so every `test-udfs/*` `.so` must be rebuilt before integration tests run — noted in the plan's checklist.
- **Promotes to ADR:** no

## Review Findings

<!-- No adversarial plan-review pass was run for this plan. Code-review findings are appended by /speq:implement. -->
