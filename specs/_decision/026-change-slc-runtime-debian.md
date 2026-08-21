# Decisions: change-slc-runtime-debian

## ADR: Replace the Alpine runtime with a curated debian:trixie-slim staged tree

**ID:** debian-trixie-slim-staged-runtime
**Plan:** change-slc-runtime-debian
**Status:** Accepted
**Supersedes:** alpine-image-musl-client-binary

### Context

The Alpine SLC never used Alpine for anything but the filesystem envelope: the shipped `exaudfclient` has been a glibc binary since the pivot recorded in ADR 005, and the image already bundled glibc runtime libraries copied out of the Debian builder. What remained was a musl userland the UDF never uses, wrapped around a glibc runtime the UDF entirely depends on — two libc worlds in one tarball, with the loader path threaded through Alpine's non-usr-merged `/lib` by hand.

### Decision

The SLC is built from a single root `Dockerfile` in three stages — a `rust:1.94-trixie` builder, a `debian:trixie-slim` donor/packager that stages a curated `/slc` tree, and a `FROM scratch` artifact stage. The staged tree contains only the glibc runtime, the documented UDF library surface, the client, `build_info/` and the notice bundles: no shell, package manager, coreutils, Rust toolchain or vendored registry.

### Options Considered

| Option | Verdict |
|--------|---------|
| `debian:trixie-slim`-staged `/slc` tree, glibc runtime + documented library surface only | ✓ Chosen — the extracted tree is the UDF's root filesystem, so its contents are a deliberate product decision; staging explicitly removes the two-libc split and lets the loader path fall out of the donor's own usr-merge layout |
| Keep the Alpine envelope with glibc bundled inside it | ✗ Rejected — ships a musl userland the UDF never uses, wrapped around the glibc runtime the UDF entirely depends on, and forces the loader to be threaded through Alpine's non-usr-merged `/lib` by hand |
| Flatten the whole `debian:trixie-slim` rootfs as the Alpine build flattened Alpine's | ✗ Rejected — ships a shell, `apt` and coreutils no UDF uses, keeps the shipped surface unreviewable, and drags GPL-2.0-only and BSD-2-Clause attribution obligations along with it |

### Consequences

The shipped artifact and its compliance surface shrink, and the loader path derivation follows the donor's own layout instead of a hand-threaded Alpine path. This supersedes ADR 005's "Alpine image — build the client binary for x86_64-unknown-linux-musl" decision, whose musl rationale was already abandoned in practice by the glibc-bundling pivot its own Consequences section records; this decision completes that pivot by removing the Alpine layer entirely.

## ADR: Runtime locale stays LANG=C.UTF-8, re-homed to Debian with the locale data staged

**ID:** debian-staged-c-utf-8-locale
**Plan:** change-slc-runtime-debian
**Status:** Accepted
**Supersedes:** alpine-runtime-lang-c-utf-8

### Context

`C.UTF-8` gives the UTF-8 string semantics UDF text handling needs, and that reason survives the base change unaltered — only the justification "musl has no `locales` package" no longer applies. The image-level `ENV` does not survive tarball extraction, so the staged locale data, not the `ENV` line, is what makes the locale resolvable inside the UDF sandbox.

### Decision

Keep `ENV LANG=C.UTF-8` on the staging stage and additionally stage `/usr/lib/locale/C.utf8` into the tree. No locale package is installed and no `locale-gen` runs.

### Options Considered

| Option | Verdict |
|--------|---------|
| `ENV LANG=C.UTF-8` plus staged `/usr/lib/locale/C.utf8` | ✓ Chosen — `debian:trixie-slim` already ships the compiled `C.utf8` locale, and staging the data (not just the `ENV`) is what survives tarball extraction |
| Install Debian's `locales` and generate `en_US.UTF-8` | ✗ Rejected — adds weight for no UDF-visible benefit |

### Consequences

The staged tree carries the locale data it needs without a `locales` package or `locale-gen` step. This supersedes ADR 005's "Alpine runtime uses LANG=C.UTF-8 instead of locale-gen" decision, whose Alpine/musl-specific rationale no longer applies to the Debian-staged runtime.

## ADR: Derive every architecture-dependent path from the donor; hand the derived values across stages in files

**ID:** derive-architecture-paths-from-donor
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

A single Dockerfile must produce a correct tree on both x86_64 and aarch64 runners with no cross-compilation. `debian:trixie-slim` carries neither `binutils` nor `dpkg-architecture`, so the donor stage cannot re-derive the multiarch triplet or the loader path itself, and a hardcoded symlink set is an arch-specific defect waiting for the other runner (x86_64 needs `/lib64`; aarch64 has none).

### Decision

The multiarch triplet and the client's `PT_INTERP` loader path are derived in the builder and written to `/slc-meta/` for the staging stage to read. The staged usr-merge layout is derived by reading which of `/lib`, `/lib64`, `/bin`, `/sbin` are symlinks in the donor and reproducing those with the donor's own targets. Symlink creation precedes file staging so all real files land under `/slc/usr`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Derive in builder, hand off via files, reproduce donor's own symlink set | ✓ Chosen — the only stage with `binutils`/`dpkg-architecture` derives the values; the donor's own layout is arch-correct by construction |
| Re-derive both values in the donor | ✗ Rejected — `debian:trixie-slim` has neither `binutils` nor `dpkg-architecture` |
| Hardcode the per-architecture symlink set | ✗ Rejected — x86_64 needs `/lib64` and aarch64 has none, so a hardcoded list is an arch-specific defect |

### Consequences

Ordering is subtle but explicit: `/usr/lib64/ld-linux-x86-64.so.2` is itself a symlink into `/usr/lib/<triplet>/`, so `cp -L` of the `PT_INTERP` path only resolves correctly once `/slc/lib64 -> usr/lib64` exists over a real `/slc/usr/lib64` directory.

## ADR: Ship the "variant E" library surface and make vendoring the contract for everything else

**ID:** slc-variant-e-library-surface
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

A UDF using `native-tls` or a compression `-sys` crate would fail at `dlopen` with a raw loader error and no diagnosis path if the SLC staged only the client's own `ldd` closure. Staging the donor's full library surface, by contrast, costs ~29 MB raw for a set nobody enumerated, versus 8.2 MB raw / 1.1 MB gzipped (measured on aarch64) for a curated set.

### Decision

Stage OpenSSL 3 (with `ossl-modules` and `engines-3`), `zlib`, `bzip2` and `zstd` beyond the client's own `ldd` closure, alongside the glibc runtime, its compatibility stubs, and the dlopen-only NSS/resolver modules. Anything a UDF links dynamically outside that surface must be vendored into the `.so`, and `cargo exasol-udf validate` reports violations.

### Options Considered

| Option | Verdict |
|--------|---------|
| Curated surface (glibc + NSS/resolver + OpenSSL + zlib/bzip2/zstd) | ✓ Chosen — these four libraries are what real `-sys` crates reach for; naming them makes the boundary a published, checkable contract |
| Stage only the client's `ldd` closure | ✗ Rejected — a UDF using `native-tls` or a compression `-sys` crate would fail at `dlopen` with no diagnosis path |
| Stage the donor's full library surface | ✗ Rejected — ~29 MB raw for a set nobody enumerated, versus 8.2 MB raw / 1.1 MB gzipped for the curated set |

### Consequences

The library surface is a published contract paired with a build-time check, so authors learn about a violation on their own machine instead of in a UDF failure.

## ADR: exaudfclient links no bzip2 at all; drop the libbz2-dev pin and its CI assertion

**ID:** exaudfclient-no-bzip2-link
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

The plan originally assumed `exaudfclient` links bzip2, dynamically or statically, with only the link mode undetermined. Verified via `readelf` on three independent builds (a fresh `rust:1.94-trixie` build, the previously shipped Alpine-based artifact, and a local host build): `exaudfclient`'s `DT_NEEDED` set and `.dynsym` carry zero bzip2 references in any of the three. Traced in the `exarrow-rs` 0.13.0 source, its only use of the `bzip2` crate gates CSV `IMPORT`/`EXPORT` local-file compression — a code path this project's `ExaConnection` usage (`query`/`query_for_each`/`execute` only) never reaches, so Rust's dead-code elimination drops the unit at link time on both profiles and both architectures.

### Decision

Do not install `libbz2-dev` in the builder, and do not assert a bzip2 `DT_NEEDED` entry on the shipped `exaudf/exaudfclient` anywhere in CI. `libbz2.so.1` stays staged in `/slc` — that guarantee is for UDF authors' own crates that link `bzip2-sys` dynamically, not for the client binary.

### Options Considered

| Option | Verdict |
|--------|---------|
| Drop the pin and the CI assertion; keep `libbz2.so.1` staged for authors | ✓ Chosen — a CI assertion should verify a real property of the shipped artifact; this one can never be true given the current `ExaConnection` surface |
| Keep `libbz2-dev` as a dormant pin against a future reachable code path | ✗ Rejected — it currently does nothing observable and invites the same false "this is exercised" reading that caused the original mistake |
| Force a reachable bzip2 call via a fixture UDF so the original assertion becomes true | ✗ Rejected — pure scope creep to satisfy a test, not a real UDF need |

### Consequences

CI no longer asserts a `DT_NEEDED` entry that could never appear. The compression-library staging story is unaffected: `libbz2.so.1` ships because real UDF `-sys` crates reach for it, independent of the SLC's own client.

## ADR: The glibc floor is 2.41, measured on this plan's own image pair, and lives in one committed file

**ID:** glibc-floor-241-single-source
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

`debian:trixie-slim` ships Debian GLIBC `2.41-12+deb13u3`. The floor is a property of the runtime distro rather than of the Rust toolchain — the toolchain version governs what the client references, not what the container provides — so the issue's inherited "2.41" figure, measured under a different variant/toolchain combination, had to be re-derived for the actual image pair (`rust:1.94-trixie` builder, `debian:trixie-slim` donor) before being published to authors.

### Decision

Record the floor as `2.41` in `crates/cargo-exasol-udf/slc-glibc-floor.txt`, read it from the CLI via `include_str!`, and have the tarball contract test assert that the committed value equals the highest `GLIBC_x.y` version the staged `libc.so.6` defines and that the shipped client references nothing above it.

### Options Considered

| Option | Verdict |
|--------|---------|
| Re-derive and commit the floor as a single machine-checkable file, verified against the shipped `libc.so.6` | ✓ Chosen — recording it once and verifying it against what ships means the published number can never drift from the container |
| Inherit the issue's "2.41" figure without re-deriving it | ✗ Rejected — that number was measured under a different variant/toolchain combination than this plan's own image pair |
| A `const` in the CLI source | ✗ Rejected — the pipeline would then have to grep Rust source to check for drift |
| Read the floor from the tarball at author time | ✗ Rejected — authors do not have the tarball |

### Consequences

The floor authors read can never drift from what ships, so a future toolchain bump cannot silently invalidate it.

## ADR: validate errors above the floor, warns on unknown dependencies, and reads the ELF once

**ID:** validate-elf-severity-tiers
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

The severity of each `cargo exasol-udf validate` check follows whether the artifact can load at all: an artifact above the glibc floor cannot load, but an existing artifact linking an unstaged library may still work on today's full Alpine userland, so failing it on day one would block authors before they can add a vendoring feature. Running the platform checks before `dlopen` also makes them testable with a generated fixture whose stub `libc.so.6` names a version above the floor.

### Decision

`validate` performs one ELF read yielding the entry symbols, the `DT_NEEDED` sonames and the highest `GLIBC_x.y` reference. An artifact above the floor is a hard error; a `DT_NEEDED` entry outside the SLC surface is a warning that `--deny-unknown-deps` escalates to an error. The `nm` shell-out is removed in the same change. Checks run ELF read → entry symbols → glibc floor → `DT_NEEDED` → dlopen ABI/fingerprint.

### Options Considered

| Option | Verdict |
|--------|---------|
| Error on the floor, warn (with an opt-in escalation) on unknown dependencies, single ELF read via `goblin` | ✓ Chosen — matches whether the artifact can load at all; `goblin` surfaces both `DT_NEEDED` and `.gnu.version_r` directly and is already MIT-allowed |
| Warn on both | ✗ Rejected for the floor — such an artifact cannot load, and raising the floor from ~2.36 to 2.41 means no previously-valid artifact starts failing |
| Error on both | ✗ Rejected for dependencies — an existing artifact may link an unstaged library and still work today, so failing it on day one blocks authors before they can vendor |
| Keep `nm` for symbols and parse only for the new checks | ✗ Rejected — two mechanisms reading the same dynamic section put ELF knowledge in two places, and dropping the shell-out removes the binutils requirement from the author's host |

### Consequences

`validate` runs no shell-out and requires no binutils on the author's host; the ELF read is a single mechanism serving all platform checks.
