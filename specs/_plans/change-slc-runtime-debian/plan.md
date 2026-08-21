# Plan: change-slc-runtime-debian

## Summary

Replace the Alpine SLC runtime with a curated `debian:trixie-slim`-staged root filesystem built from a single root `Dockerfile`, purging every live Alpine/musl reference from the repository. The same change regenerates the OS-layer license bundle for the new library surface, and teaches `cargo exasol-udf validate` to check an artifact's glibc symbol floor and dynamic-dependency set against what the container actually provides.

## Design

### Context

The Alpine SLC never used Alpine for anything but the filesystem envelope: the shipped `exaudfclient` has been a glibc binary since the pivot recorded in ADR 005, and the image already bundled glibc runtime libraries copied out of the Debian builder. What remained was a musl userland the UDF never uses, wrapped around a glibc runtime the UDF entirely depends on — two libc worlds in one tarball, with the loader path having to be threaded through Alpine's non-usr-merged `/lib` by hand.

The spike behind issue #84 settled the replacement. Three facts drive this design:

- The extracted SLC tree **is** the UDF's root filesystem. What a UDF `.so` may link dynamically is this project's decision, not the Exasol host's.
- The glibc constraint is **per-symbol, not per-distro**. Bundling a newer glibc shrinks the window in which an author's host stamps an unresolvable `GLIBC_x.y` reference into an artifact; it never closes it. The remedy is to publish the floor as a machine-checkable number and check artifacts against it.
- `exaudfclient` itself links no bzip2 at all — `exarrow-rs`'s bzip2 usage lives entirely behind its CSV `IMPORT`/`EXPORT` local-file-compression feature, unreachable from this project's `ExaConnection` usage, so the linker drops it. `libbz2.so.1` stays staged in `/slc` for UDF authors' own crates regardless (see decision-log entry 5).

- **Goals** — one Dockerfile; a staged tree containing only what UDFs need; a published, enforced glibc floor; author-facing detection of unloadable artifacts before upload; no live Alpine/musl vocabulary left in the repository.
- **Non-Goals** — the Rust toolchain bump to 1.97 (a separate PR); multi-node verification (already open for the current SLC); aarch64 integration tests against a live database (`exasol/docker-db` is amd64-only); making the SLC surface configurable per deployment.

### Decision

#### Architecture

```
┌──────────────────────────────┐
│ builder  rust:1.94-trixie    │  cargo build --release -p exaudfclient
│                              │  apt: protobuf-compiler pkg-config libbz2-dev
│  derives, never hardcodes:   │  (no libbz2-dev: exaudfclient links no
│   TRIPLET = gcc -print-…     │   bzip2 at all — see decision-log entry 5)
│   LOADER  = PT_INTERP        │  writes both to /slc-meta/ (donor has no
└───────────────┬──────────────┘  binutils and no dpkg-architecture)
                │ binary + /slc-meta
                ▼
┌──────────────────────────────┐
│ staging  debian:trixie-slim  │  apt: ca-certificates tzdata (only)
│  donor AND packager          │
│                              │  1. reproduce own usr-merge symlinks in /slc
│   /slc/usr/...  real files   │     (lib, lib64, bin, sbin — whichever are
│   /slc/lib   -> usr/lib      │      symlinks in the donor)
│   /slc/lib64 -> usr/lib64    │  2. cp -L the library surface + loader at
│   /slc/bin   -> usr/bin      │     its own PT_INTERP path
│   /slc/sbin  -> usr/sbin     │  3. payload: exaudfclient, build_info,
│                              │     LICENSE, both notice bundles
│                              │  4. /etc/{hosts,resolv.conf} -> /conf/…
│                              │  5. chroot /slc /exaudf/exaudfclient
│                              │     ⇒ must report wrong argument count
│                              │  6. tar --hard-dereference -> lc-rs.tar.gz
└───────────────┬──────────────┘
                │ lc-rs.tar.gz
                ▼
┌──────────────────────────────┐
│ artifact  FROM scratch       │  docker build --target artifact --output
└──────────────────────────────┘
```

The container contract is then enforced from two sides that never trust each other:

```
crates/cargo-exasol-udf/slc-glibc-floor.txt   ← single committed value
        │                                │
        │ include_str!                   │ read by shell
        ▼                                ▼
  slc_surface.rs                 dist/tests/slc_tarball_test.sh
  (CLI: what the SLC provides)   (asserts the value equals the highest
        │                         GLIBC_x.y the staged libc.so.6 defines)
        ▼
     validate.rs  ──uses──▶  elf.rs  (one ELF read: entry symbols,
                                      DT_NEEDED, max GLIBC_x.y)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Derive-from-donor, never hardcode | `Dockerfile` usr-merge loop, multiarch triplet, `PT_INTERP` | The same Dockerfile must build correct trees on x86_64 (which has `/lib64`) and aarch64 (which does not); a hardcoded path is an arch-specific defect waiting for the other runner |
| Cross-stage handoff via files | builder writes `/slc-meta/{triplet,loader}` | `debian:trixie-slim` has neither `binutils` nor `dpkg-architecture`, so the values must be derived where the tools live and carried, not re-derived |
| Single owner + independent enforcement | `slc-glibc-floor.txt` read by the CLI, verified by the tarball test against the shipped `libc.so.6` | The floor appears once; the pipeline proves it still matches what ships, so the CLI cannot report a stale number |
| One deep ELF read | `crates/cargo-exasol-udf/src/elf.rs` | Entry symbols, `DT_NEEDED` and glibc version-needed all come from the same dynamic section; splitting them across an `nm` shell-out and a parser puts ELF knowledge in two places |
| Contract test over the shipped artifact | `dist/tests/slc_tarball_test.sh` | Asserting the builder's package list proves nothing about the tarball; asserting the tarball catches every way the build could regress, and runs on both architectures |
| Warn-then-opt-in-deny | `validate --deny-unknown-deps` | Existing artifacts may carry `DT_NEEDED` entries that happen to resolve on today's full Alpine userland; failing them on day one blocks authors before they can vendor |

Both new CLI modules are justified against the design-philosophy Quick Diagnostic. `elf.rs` is deep: callers pass a path and receive three derived facts, never an ELF concept or the parsing crate — the interface is far smaller than the work it absorbs, and `validate.rs` and `build.rs` both shed their ELF knowledge to it. `slc_surface.rs` is small but owns a decision that would otherwise leak into `validate.rs`, the tarball test and the docs independently: *what the SLC provides*. Splitting them keeps ELF-format knowledge and container-contract knowledge in separate owners, which is the axis along which each changes.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| One unqualified root `Dockerfile` | Keep `Dockerfile.debian` / add `Dockerfile.trixie` | With Alpine gone there is nothing to disambiguate; a qualified name invites a second variant nobody maintains, which is how the phantom `Dockerfile.debian` reference in `mission.md` and `architecture.md` survived |
| Curated staged tree, no rootfs flatten | Flatten `debian:trixie-slim`'s whole rootfs as the Alpine build flattened Alpine's | The flatten shipped a shell, package manager and coreutils the UDF never uses; staging explicitly makes the shipped surface reviewable and smaller, and removes every GPL-2.0-only and BSD-2-Clause attribution obligation |
| Stage the "variant E" surface beyond the client's own `ldd` closure | Stage only the closure and let authors vendor OpenSSL/compression | +8.2 MB raw / +1.1 MB gzipped (measured on aarch64) buys UDFs OpenSSL and the three compression libraries — the dependencies real `-sys` crates reach for — while staying far below the ~29 MB of the donor's full library surface |
| `debian:trixie-slim` donates the runtime libraries | Keep donating from the `rust:1.94-trixie` builder | The builder carries `-dev` packages whose symlinks and versions drift with the toolchain image; the slim donor is a clean, minimal, auditable set. Both are Debian 13, so the glibc is identical (verified: `2.41-12+deb13u3`) |
| No `libbz2-dev` in the builder; no bzip2 CI assertion | Pin `libbz2-dev` for a deterministic dynamic link (the plan's original assumption); force a reachable bzip2 call to make that assertion true | `exaudfclient` links no bzip2 at all — `exarrow-rs`'s bzip2 usage is confined to its CSV `IMPORT`/`EXPORT` local-file-compression path, unreachable from this project's `ExaConnection` usage (see decision-log entry 5). A pin or assertion on an artifact property that can never appear either always fails or requires manufacturing a fake call site; `libbz2.so.1` stays staged in `/slc` for UDF authors regardless |
| Glibc floor in a committed text file | A `const` in the CLI; read from the tarball at author time | The tarball is not available on an author's machine. A file lets the CLI and the pipeline read the *same* bytes; the pipeline then proves the value against the shipped `libc.so.6`, so the two can never drift |
| Above-floor glibc reference is an error | Warn only | Such an artifact cannot load — the loader rejects it eagerly. Raising the floor from 2.36 to 2.41 only makes *more* existing artifacts pass, so nothing that validated before starts failing |
| Unknown `DT_NEEDED` warns by default | Error by default | An artifact linking an unstaged library may still work on today's Alpine userland; erroring on day one breaks `validate` for authors before they can add a vendoring feature. `--deny-unknown-deps` gives CI the strict mode |
| Replace the `nm` shell-out while adding the parser | Keep `nm` for symbols, parse only for the new checks | Two mechanisms reading the same ELF is back-door leakage — the format would be known in two places. One read drops the binutils runtime requirement as a side effect |
| The chroot self-test stays in the Docker build | Move it into the tarball test | Inside the build we are root by construction; the tarball test would need `sudo` to `chroot`, which is neither available nor silent-skip-safe. The tarball test asserts the equivalent structural property (the `PT_INTERP` path resolves inside the tree) without root |
| Reword ADR 025's usr-merge clause; add a pointer line to ADR 024 | Rewrite both ADR bodies | ADR 025's clause is a *live* normative platform fact the ADR exists to preserve, so it must track reality. Nothing in ADR 024's body is Alpine-specific or falsified by this change, so it gets a pointer, not a rewrite — see decision-log entry 8 |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/slim-image | CHANGED | `specs/_plans/change-slc-runtime-debian/container/slim-image/spec.md` |
| container/os-license-notices | CHANGED | `specs/_plans/change-slc-runtime-debian/container/os-license-notices/spec.md` |
| container/crate-license-notices | CHANGED | `specs/_plans/change-slc-runtime-debian/container/crate-license-notices/spec.md` |
| tools/cargo-exaudf | CHANGED | `specs/_plans/change-slc-runtime-debian/tools/cargo-exaudf/spec.md` |

## Impact

**Breaking for anyone building the SLC by file name.** `Dockerfile.alpine` is deleted; every build invocation becomes `docker build --target artifact` against the root `Dockerfile`. In-repo callers (`ci.yml`, `scripts/install.sh`, `scripts/ci-it-local.sh`, `benches/README.md`, the `it` harness error text) are updated in this plan; any out-of-tree script or downstream fork pinning `-f Dockerfile.alpine` must change.

**Breaking for a UDF that expects a userland inside the sandbox.** The staged tree has no shell, no package manager and no coreutils, so a UDF that shells out to `/bin/sh` or a coreutils binary stops working. No documented behaviour permitted this, and nothing in-tree relies on it.

**Release assets are unchanged** — still `lc-rust-<version>.tar.gz` and `lc-rust-<version>-aarch64.tar.gz`, same layout, same `language_definitions.json`, same `SCRIPT_LANGUAGES` fragment. Registration needs no change and the tarball remains drop-in replaceable.

**Widened author compatibility.** The bundled glibc moves from ~2.36 to 2.41, so `.so` files built on hosts up to glibc 2.41 (Debian 13, Ubuntu 25.04) now load where they previously failed at `dlopen`. No artifact that loaded before stops loading.

**`validate` gains platform checks.** It now also reports the artifact's highest `GLIBC_x.y` reference and its dynamic dependencies. An artifact above the floor becomes a hard failure (it could never load); a dependency outside the SLC surface is a warning, escalated to a failure only with the new `--deny-unknown-deps`. `validate` no longer requires `nm`/binutils on the author's host.

## Requirements

| Requirement | Details |
|-------------|---------|
| Verified base images | `rust:1.94-trixie` and `debian:trixie-slim` both exist for `amd64` and `arm64` (manifest-verified) |
| Verified glibc floor | `debian:trixie-slim` ships Debian GLIBC `2.41-12+deb13u3` → floor `2.41`. See decision-log entry 6 — this supersedes the issue's inherited "2.41 measured under Rust 1.97" figure with a measurement of the actual PR1 image pair |
| Verified library surface | `libssl.so.3`, `libcrypto.so.3`, `libz.so.1`, `libbz2.so.1`, `libzstd.so.1`, `libstdc++.so.6`, `libgcc_s.so.1`, `libresolv.so.2`, `libnss_files.so.2`, `libnss_dns.so.2`, the `libpthread`/`libdl`/`librt` stubs, `ossl-modules/legacy.so`, `engines-3/{afalg,loader_attic,padlock}.so` and `/usr/lib/locale/C.utf8` are all already present in `debian:trixie-slim` — only `ca-certificates` and `tzdata` need `apt-get install` |
| Verified closure | Every staged library's `DT_NEEDED` set is inside the staged surface (`libssl`→`libcrypto,libz,libzstd,libc`; `libcrypto`→`libz,libzstd,libc`; `libstdc++`→`libm,libc,libgcc_s`) |
| Do not apt-install the libraries | Adding `libssl3` to the apt list fails on trixie — the package was renamed `libssl3t64` in the time64 transition. Because every staged library is already in the base image, no package name for them ever appears in the Dockerfile, which sidesteps that trap entirely |
| Verified source packages for the notice bundle | `libc6 2.41-12+deb13u3` → `glibc`; `libgcc-s1`/`libstdc++6 14.2.0-19` → `gcc-14` (not `gcc-12`); `libssl3t64 3.5.6` → `openssl`; `zlib1g` → `zlib`; `libzstd1` → `libzstd`; `libbz2-1.0 1.0.8-6` → `bzip2` |
| PR2 excluded | The Rust toolchain stays `1.94`; `rust-toolchain.toml` is untouched and the builder keeps the `rm rust-toolchain.toml` pattern rather than `RUSTUP_TOOLCHAIN` |

## Dependencies

- **New crate dependency** — an ELF reader for `cargo-exasol-udf`: `goblin` (MIT), added to `[workspace.dependencies]` per the workspace-centralisation rule. It exposes DT_NEEDED sonames and the `.gnu.version_r` table directly, which `object` only reaches through lower-level section walking. MIT is already on `deny.toml`'s allow list. The crate graph of `crates/exaudfclient` is untouched, so `THIRD-PARTY-LICENSES.md` does not change.
- **Unchanged external tooling** — `cargo-about@0.9.0` (already installed by the `build-slc` and `install.sh` paths), `readelf` from binutils on CI runners for the tarball assertions, Docker Buildx.

## Migration

| Current | New |
|---------|-----|
| `Dockerfile.alpine` (4 stages: builder, runtime, packager, artifact) | `Dockerfile` (3 stages: builder, staging, artifact) |
| `FROM rust:1.94-bookworm` builder | `FROM rust:1.94-trixie` builder |
| `alpine:3` runtime + `apk add ca-certificates tzdata tar` | `debian:trixie-slim` staging + `apt-get install ca-certificates tzdata` |
| Flatten the whole runtime rootfs into `/slc` | Stage only the curated surface into `/slc` |
| Glibc staged in the builder into `/glibc-rt`, copied into the runtime image | Glibc staged in the donor directly into `/slc` |
| `apk` package → SPDX table in `dist/os-licenses.hbs` | Staged-library → SPDX table |
| Alpine `aports` + Debian source offer | Debian-only source offer |
| `validate` shells out to `nm` | `validate` reads the ELF in-process |
| Bundled glibc ~2.36 (undocumented, unverified) | Bundled glibc `2.41`, recorded in `crates/cargo-exasol-udf/slc-glibc-floor.txt` and verified against the shipped `libc.so.6` |

## Implementation Tasks

1. **Shared contract**
   - [ ] 1.1 Add `crates/cargo-exasol-udf/slc-glibc-floor.txt` containing exactly the bundled glibc floor (`2.41`), documented by a doc comment at its only Rust reader — the file itself stays parse-simple so a shell test can read it too.

2. **Container build** — replaces the Alpine image
   - [ ] 2.1 Add the root `Dockerfile` builder stage on `rust:1.94-trixie`: `apt-get install protobuf-compiler pkg-config` (still no `libzmq3-dev`, and no `libbz2-dev` — `exaudfclient` links no bzip2 at all, see decision-log entry 5), keep the existing workspace `COPY` set and the `rm rust-toolchain.toml` pattern, build `-p exaudfclient`, then derive the multiarch triplet and the binary's `PT_INTERP` loader path, fail with a named error if either is empty, and record both under `/slc-meta/` for the staging stage. [expert]
   - [ ] 2.2 Add the `debian:trixie-slim` staging stage: `apt-get install ca-certificates tzdata`, `ENV LANG=C.UTF-8`, reproduce whichever of `/lib`, `/lib64`, `/bin`, `/sbin` are symlinks in the donor into `/slc` with the donor's own targets, put every real file under `/slc/usr`, then `cp -L` the glibc core and compatibility stubs, the loader at its exact `PT_INTERP` path, `libnss_files`/`libnss_dns`/`libresolv`, `libgcc_s`/`libstdc++`, `libssl`/`libcrypto` with `ossl-modules` and `engines-3`, `libz`/`libbz2`/`libzstd`, `/usr/lib/locale/C.utf8`, the zoneinfo database and the `ca-certificates` bundle with the OpenSSL default trust path that reaches it; write a minimal `/slc/etc/nsswitch.conf` naming only staged modules, plus `ld.so.conf` and `ldconfig -r /slc`. Symlink creation must precede file staging so `cp -L` writes through the reproduced links into real `/usr` directories. [expert]
   - [ ] 2.3 Stage the payload into `/slc` (`exaudfclient` at `/exaudf/exaudfclient` with the executable bit, `build_info/`, `LICENSE`, `dist/THIRD-PARTY-LICENSES.md`, `dist/THIRD-PARTY-OS-LICENSES.md`), create the `/conf/hosts` and `/conf/resolv.conf` symlinks, run the `chroot /slc /exaudf/exaudfclient` self-test asserting a non-zero exit and the wrong-argument-count message, `tar --hard-dereference` the tree into `/lc-rs.tar.gz`, and add the `FROM scratch AS artifact` stage.
   - [ ] 2.4 Delete `Dockerfile.alpine`.

3. **Tarball contract test and pipeline wiring**
   - [ ] 3.1 Add `dist/tests/slc_tarball_test.sh <tarball>`: extract to a temp dir and assert the client is present and executable; the reproduced usr-merge symlinks match the runner's expectation for its architecture; the `PT_INTERP` path resolves to a real loader file inside the tree; the documented library surface is present; every `DT_NEEDED` entry of every staged ELF resolves inside the tree; neither `libbz2.so.1` nor `libzmq` is a `DT_NEEDED` entry of the client (the client links no bzip2 at all — see decision-log entry 5); the committed floor equals the highest `GLIBC_x.y` the staged `libc.so.6` defines and the client references nothing above it; `etc/hosts` and `etc/resolv.conf` are symlinks into `/conf`; `usr/share/zoneinfo/Europe/Berlin` is a non-empty regular file; `nsswitch.conf` names only staged modules; the OpenSSL trust path resolves; `usr/lib/locale/C.utf8` is present; `build_info/language_definitions.json` declares the expected schema, protocol, alias, parameter and executable; the three notice files are present and the OS notice mentions no apk/Alpine source; no `bin/sh`, `usr/bin/apt` or `usr/bin/dpkg` exists; and the staged surface outside `exaudf/` stays under a committed size ceiling whose measured value the script prints. [expert]
   - [ ] 3.2 Wire the new test and the new Dockerfile through the pipeline: `ci.yml`'s `build-slc` job (both matrix legs) builds against `Dockerfile`, updates its adjacent step comments naming `Dockerfile.alpine`, and runs `dist/tests/slc_tarball_test.sh` on the produced tarball; `scripts/ci-it-local.sh` and `scripts/install.sh` build against `Dockerfile` (and `ci-it-local.sh` runs the tarball test); update the `docker build` lines in `benches/README.md` and the `SLC_TARBALL` hint in `crates/it/src/lib.rs`.

4. **OS license bundle for the Debian surface**
   - [ ] 4.1 Rewrite `dist/os-attribution/Cargo.toml`'s synthetic `license` expression and `dist/about-os.toml`'s `accepted` list for the staged set — LGPL-2.1, `GPL-3.0-only WITH GCC-exception-3.1`, MPL-2.0, MIT, Apache-2.0, Zlib, BSD-3-Clause, bzip2-1.0.6 — dropping GPL-2.0-only and BSD-2-Clause, which no longer ship.
   - [ ] 4.2 Rewrite `dist/os-licenses.hbs`: replace the apk package table with the staged-library → SPDX table, drop the Alpine `aports` offer and the `/lib/apk/db/installed` pointer, and update the header prose to describe a curated staged tree rather than a flattened rootfs. The three-year source offer keeps `snapshot.debian.org`/`sources.debian.org` but must name Debian 13 (trixie) and the correct source packages — `glibc`, `gcc-14` (not the current `gcc-12`), `openssl`, `zlib`, `libzstd`, `bzip2`, `ca-certificates`, `tzdata` — and the two "Debian 12 / `rust:1.94-bookworm`" headings must move to Debian 13. Update the comments in `dist/generate-licenses.sh`, and append the bzip2 license text from SPDX license-list-data alongside the GCC exception if `cargo about` does not render `bzip2-1.0.6` from its embedded store.
   - [ ] 4.3 Add `dist/tests/os_licenses_test.sh` asserting the committed boilerplate exists, that `about-os.toml`'s accepted list and the synthetic crate's `license` expression name the same set, that the generated manifest carries the staged-library table, the Debian-only source offer, the GCC exception and the bzip2 text, and that it contains no apk/Alpine/musl reference; run it in `build-slc` right after `dist/generate-licenses.sh`.

5. **`cargo exasol-udf validate` platform checks**
   - [ ] 5.1 Add `goblin` to `[workspace.dependencies]` and to `crates/cargo-exasol-udf/Cargo.toml`, and regenerate `Cargo.lock`.
   - [ ] 5.2 Add `crates/cargo-exasol-udf/src/elf.rs` (plus `elf_tests.rs`): one function that reads a path once and returns the exported `__exa_udf_entry_<NAME>` suffixes, the `DT_NEEDED` sonames and the highest `GLIBC_x.y` version referenced in `.gnu.version_r`, with a named error for a file that is not a parseable ELF shared object. No goblin type may appear in its public interface. [expert]
   - [ ] 5.3 Add `crates/cargo-exasol-udf/src/slc_surface.rs` (plus `slc_surface_tests.rs`): the SLC library allowlist, the always-allowed loader and vdso patterns, the floor read from `slc-glibc-floor.txt` via `include_str!`, and the two pure predicates — classify unknown sonames, and compare a referenced glibc version against the floor.
   - [ ] 5.4 Replace `validate::enumerate_entry_symbols`' `nm` shell-out with `elf::read`, keeping the existing signature so `build.rs`'s artifact check keeps working, and drop the now-dead binutils error path. [expert]
   - [ ] 5.5 Add the platform checks to `validate::run` in the order ELF read → entry symbols → glibc floor → `DT_NEEDED` → dlopen ABI/fingerprint, parse `--deny-unknown-deps` in the hand-rolled style of `build::parse_build_args`, and extend `write_usage` in `main.rs`.
   - [ ] 5.6 Extend `crates/cargo-exasol-udf/tests/validate.rs`: a fixture whose reported glibc summary is asserted; a generated stub `libc.so.6` (soname override plus a version script defining a `GLIBC_` version above the floor) that must be rejected; a generated unstaged `libwidget.so` the fixture links against, asserted to warn and exit zero, then to fail under `--deny-unknown-deps`; a non-ELF input reported as such; and the existing multiarch candidate paths rebuilt from `std::env::consts::ARCH` with a loud failure instead of the current silent `SKIP:`. [expert]

6. **Purge the Alpine/musl vocabulary from live artifacts**
   - [ ] 6.1 `specs/mission.md` — fix the Tech Stack "Container base" row to name the single `Dockerfile` on `debian:trixie-slim`; `specs/architecture.md` — collapse the two Dockerfile tree entries (including the phantom `Dockerfile.debian`) into one.
   - [ ] 6.2 `docs/writing-a-udf.md` — the builder image, the glibc floor (2.41, naming `slc-glibc-floor.txt` as the machine-readable source), the SLC library surface and the vendor-everything-else rule, and the new `validate` checks; `docs/installation.md` — the release-asset paragraph's Dockerfile reference.
   - [ ] 6.3 `CLAUDE.md` — remove the moot Alpine-vs-Debian connect-back bullet and record the facts worth keeping in its place (single root `Dockerfile`; staged tree has no shell; the library surface and floor contract).
   - [ ] 6.4 `crates/it/tests/db_roundtrip.rs` — reword the tzdata scenario comment that ties the regression to "an Alpine image built without tzdata". Leave the deliberate musl-absence guards in `about.toml`, `dist/tests/about_toml_test.sh` and `crates/cargo-exasol-udf/tests/build.rs` untouched.
   - [ ] 6.5 ADRs — reword ADR 025's PT_INTERP/usr-merge parenthetical so the preserved platform fact matches the staged tree (leave its "integration tests stay x86_64-only" clause alone), and add a one-line pointer to ADR 024 noting that the bundled glibc moved to Debian 13 while the artifact model itself did not change. Change no other ADR body text.

7. **Release hygiene**
   - [ ] 7.1 Bump `[workspace.package].version` to `0.23.0`, track it in the pinned `exasol-udf-sdk` `[workspace.dependencies]` entry, and commit the regenerated `Cargo.lock`. The bump changes `EXA_SDK_FINGERPRINT`, so every `test-udfs/*` `.so` must be rebuilt before integration tests run.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 2.1, 2.2, 2.3, 2.4 (sequential within the group — one file) |
| Group B | 4.1, 4.2, 4.3 |
| Group C | 5.1, 5.2, 5.3, 5.4, 5.5, 5.6 (sequential within the group) |
| Group D | 3.1, 3.2 |
| Group E | 6.1, 6.2, 6.3, 6.4, 6.5 |

Sequential dependencies:
- 1.1 → Group A, Group C, Group D (all three read the committed floor)
- Group A → Group D (the tarball test needs a tarball to assert against, and 3.2 rewires the same build invocations)
- Group A, Group B, Group C → Group E (the docs and specs describe the finished Dockerfile, license bundle and CLI behaviour)
- Group D, Group E → 7.1 (version bump last, so `Cargo.lock` is regenerated once)
- Group B is independent of Groups A and C and may run concurrently with either.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| File | `Dockerfile.alpine` | Replaced by the root `Dockerfile` |
| Stage | The Alpine `runtime` and `packager` stages' rootfs-flatten `tar` pipeline | Replaced by explicit staging into `/slc` |
| Shell-out | `nm --dynamic --defined-only` in `crates/cargo-exasol-udf/src/validate.rs`, and its "install binutils and retry" error path | Replaced by in-process ELF reading |
| Table | The apk package → SPDX table in `dist/os-licenses.hbs` | No apk package ships |
| Section | The Alpine `aports` written-source offer and `/lib/apk/db/installed` pointer in `dist/os-licenses.hbs` | No apk package ships |
| Licenses | `GPL-2.0-only` and `BSD-2-Clause` in `dist/about-os.toml` and `dist/os-attribution/Cargo.toml` | Their only sources were Alpine's busybox/apk-tools and musl-utils |
| Spec scenarios | `container/slim-image`: "docker build produces a tagged slim image", "Binary runs and reports its usage in the image", "Alpine runtime stage is slim and self-sufficient", "Alpine image passes the db-roundtrip integration suite", "Alpine image is smaller than the Debian slim image" | The first two assumed a runnable runtime image the staged build no longer produces; the last three are Alpine-specific, and the duplicate "slim and self-sufficient" pair collapses into one |
| Spec scenario | `container/os-license-notices`: "Dockerfile.alpine ships the generated OS-license manifest into the tarball" | Renamed with the Dockerfile |
| Tree entry | `Dockerfile.debian` row in `specs/architecture.md` and `specs/mission.md` | Names a file that has never existed in this tree |
| Doc bullet | `CLAUDE.md` "Alpine vs Debian SLC image makes no difference to connect-back" | Only one image exists |
| Test branch | The silent `SKIP:` fallback in `crates/cargo-exasol-udf/tests/validate.rs` | Hid the loss of coverage on aarch64 |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| slim-image / docker build produces the SLC artifact tarball | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_contains_executable_client` |
| slim-image / Builder toolchain and glibc runtime | Integration | `dist/tests/slc_tarball_test.sh` | `slc_client_dt_needed_is_expected_set` |
| slim-image / SLC builds natively for the host architecture | Integration | `dist/tests/slc_tarball_test.sh` | `slc_client_matches_host_arch_and_loader_resolves` |
| slim-image / Staged tree reproduces the donor's usr-merge layout | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_usr_merge_symlinks_match_arch` |
| slim-image / Runtime stage is slim and self-sufficient | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_has_no_shell_or_package_manager`, `slc_tarball_has_c_utf8_locale` |
| slim-image / Staged tree provides the documented UDF library surface | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_library_surface_present`, `slc_tarball_dt_needed_closure_is_complete`, `slc_tarball_nsswitch_modules_are_staged`, `slc_tarball_openssl_trust_path_resolves` |
| slim-image / Staged glibc defines the documented author floor | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_glibc_floor_matches_committed_value` |
| slim-image / Language definitions file is present and well-formed | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_language_definitions_well_formed` |
| slim-image / Staged tree passes an in-build chroot self-test | Integration | `Dockerfile` staging stage, exercised by `docker build --target artifact` in `.github/workflows/ci.yml` `build-slc` | in-build `chroot` self-test (fails the build) |
| slim-image / Staged tree passes an in-build chroot self-test | Integration | `dist/tests/slc_tarball_test.sh` | `slc_client_matches_host_arch_and_loader_resolves` |
| slim-image / Debian-staged SLC passes the db-roundtrip integration suite | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` |
| slim-image / Staged tarball carries only the curated runtime surface | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_has_no_shell_or_package_manager`, `slc_tarball_staged_surface_within_ceiling` |
| slim-image / SLC tarball ships the /conf resolver symlinks | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_conf_resolver_symlinks` |
| slim-image / Runtime image bundles the IANA zoneinfo database | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_zoneinfo_is_regular_file` |
| os-license-notices / OS-layer license generator boilerplate is committed under dist/ | Integration | `dist/tests/os_licenses_test.sh` | `os_boilerplate_committed_and_license_sets_agree` |
| os-license-notices / The generator renders a complete OS-license manifest via cargo-about | Integration | `dist/tests/os_licenses_test.sh` | `os_manifest_covers_staged_library_set` |
| os-license-notices / The Dockerfile ships the generated OS-license manifest into the tarball | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_carries_notice_bundles` |
| os-license-notices / Distributed tarball carries the OS-layer notice at /exaudf | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_carries_notice_bundles`, `slc_tarball_os_notice_has_no_apk_references` |
| crate-license-notices / Target set reflects the shipped glibc binary | Integration | `dist/tests/about_toml_test.sh` | `about_toml_lists_gnu_triples` |
| crate-license-notices / Generated manifest ships in the tarball for each architecture | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_carries_notice_bundles` |
| cargo-exaudf / validate accepts a compatible .so | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_accepts_named_entries_and_reports_platform_summary` |
| cargo-exaudf / validate rejects a .so missing any entry symbol | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_missing_entry_symbol`, `validate_rejects_non_elf_input` |
| cargo-exaudf / validate reports the artifact's glibc version floor | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_reports_glibc_floor_summary` |
| cargo-exaudf / validate reports the artifact's glibc version floor | Unit | `crates/cargo-exasol-udf/src/elf_tests.rs` | `max_glibc_version_picks_highest_reference`, `max_glibc_version_is_none_without_verneed` |
| cargo-exaudf / validate rejects an artifact above the SLC glibc floor | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_glibc_above_floor` |
| cargo-exaudf / validate rejects an artifact above the SLC glibc floor | Unit | `crates/cargo-exasol-udf/src/slc_surface_tests.rs` | `floor_check_rejects_newer_glibc`, `committed_floor_parses` |
| cargo-exaudf / validate warns on dynamic dependencies outside the SLC library surface | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_warns_on_unknown_dt_needed` |
| cargo-exaudf / validate warns on dynamic dependencies outside the SLC library surface | Unit | `crates/cargo-exasol-udf/src/slc_surface_tests.rs` | `unknown_sonames_exclude_loader_and_vdso` |
| cargo-exaudf / validate escalates unknown dynamic dependencies on request | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_denies_unknown_dt_needed_with_flag`, `validate_allows_staged_dt_needed_under_flag` |

Unit tests appear only where the assertion is a pure comparison over already-parsed data (version-string ordering, allowlist classification, floor parsing); every scenario also carries an integration test that drives the built artifact.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/slim-image | `bash dist/generate-licenses.sh && mkdir -p /tmp/slc && docker build --target artifact --output type=local,dest=/tmp/slc .` | Build succeeds, including the in-build `chroot` self-test; `/tmp/slc/lc-rs.tar.gz` exists |
| container/slim-image | `bash dist/tests/slc_tarball_test.sh /tmp/slc/lc-rs.tar.gz` | Every assertion passes; the staged-surface size and the detected glibc floor (`2.41`) are printed |
| container/slim-image | `tar -tvzf /tmp/slc/lc-rs.tar.gz \| grep -E ' \./(etc/(hosts\|resolv.conf)\|lib\|lib64\|bin\|sbin) '` | `etc/hosts -> /conf/hosts`, `etc/resolv.conf -> /conf/resolv.conf`, and the usr-merge entries as symlinks |
| container/slim-image | `SLC_TARBALL=/tmp/slc/lc-rs.tar.gz cargo test -p it --features integration` | 0 failures across the db-roundtrip suite, including the name-resolution and session-timezone scenarios |
| container/os-license-notices | `bash dist/tests/os_licenses_test.sh` | Passes; no `apk`, `alpine` or `musl` string in `dist/THIRD-PARTY-OS-LICENSES.md` |
| container/os-license-notices | `grep -c 'GCC Runtime Library Exception\|bzip2' dist/THIRD-PARTY-OS-LICENSES.md` | Non-zero for both texts |
| container/crate-license-notices | `bash dist/tests/about_toml_test.sh && tar -tzf /tmp/slc/lc-rs.tar.gz \| grep THIRD-PARTY` | Passes; both `exaudf/THIRD-PARTY-LICENSES.md` and `exaudf/THIRD-PARTY-OS-LICENSES.md` listed |
| tools/cargo-exaudf | `cargo build --release -p scalar-double && cargo run -p cargo-exasol-udf -- exasol-udf validate target/release/libscalar_double.so` | Exit 0; reports the UDF name, the highest `GLIBC_x.y` reference against floor `2.41`, and the dynamic dependencies with no unknown-dependency warning |
| tools/cargo-exaudf | `cargo run -p cargo-exasol-udf -- exasol-udf validate --deny-unknown-deps target/release/libscalar_double.so` | Exit 0 — every dependency is inside the SLC surface |
| tools/cargo-exaudf | `cargo run -p cargo-exasol-udf -- exasol-udf validate Cargo.toml` | Exit non-zero naming `Cargo.toml` as not a parseable ELF shared object |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures (needs `SLC_TARBALL` and freshly rebuilt `test-udfs/*` `.so` files after the version bump) |
| Shell tests | `bash dist/tests/about_toml_test.sh && bash dist/tests/os_licenses_test.sh && bash dist/tests/slc_tarball_test.sh /tmp/slc/lc-rs.tar.gz` | Exit 0 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
| Licenses | `cargo deny check licenses` | Exit 0 |

### Accepted verification gaps

- **Multi-node** — open and unverified for the current SLC too; orthogonal to the runtime base.
- **aarch64 against a live database** — structurally impossible in CI (`exasol/docker-db` is amd64-only, ADR 025). Coverage on that leg is the in-build `chroot` self-test plus the full tarball contract test, both of which run natively on `ubuntu-24.04-arm`, and a manual Exasol Personal run.
- **Staged OpenSSL under load** — no in-tree UDF fixture links `libssl`/`libcrypto` dynamically (the HTTPS spike used `rustls` + `webpki-roots`), so OpenSSL is verified structurally: present, and every `DT_NEEDED` entry in its transitive closure resolves inside the staged tree. Adding a `native-tls` fixture would need a new `test-udfs` crate wired into the CI `-p` allowlist and is out of scope here.
- **musl/glibc OpenSSL mismatch on aarch64** — closed by staging glibc-compiled `libssl`/`libcrypto` from the trixie donor.
