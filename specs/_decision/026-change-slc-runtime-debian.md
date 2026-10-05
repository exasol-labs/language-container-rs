# Decisions: change-slc-runtime-debian

## ADR: Replace the Alpine runtime with a curated debian:trixie-slim staged tree

**ID:** debian-trixie-slim-staged-runtime
**Plan:** change-slc-runtime-debian
**Status:** Accepted
**Supersedes:** alpine-image-musl-client-binary

### Context

The `exaudfclient` is a glibc binary, so an Alpine base adds a musl userland the UDF never uses. Two libc worlds in one tarball also force the loader path through Alpine's non-usr-merged `/lib` by hand.

### Decision

The SLC is built from a single root `Dockerfile` in three stages: a Rust `trixie` builder, a `debian:trixie-slim` donor/packager that stages a curated `/slc` tree, and a `FROM scratch` artifact stage. The staged tree contains only the glibc runtime, the documented UDF library surface, the client, `build_info/` and the notice bundles. It has no shell, package manager, coreutils, Rust toolchain or vendored registry.

### Options Considered

| Option | Verdict |
|--------|---------|
| Curated `debian:trixie-slim` staged tree | ✓ Chosen |
| Alpine envelope with glibc bundled inside | ✗ Ships an unused musl userland and needs a hand-threaded loader path |
| Flatten the whole `debian:trixie-slim` rootfs | ✗ Ships a shell, `apt` and coreutils no UDF uses, and adds GPL-2.0-only and BSD-2-Clause attribution obligations |

### Consequences

The shipped artifact and its compliance surface are small, and the loader path follows the donor's own layout.

## ADR: Runtime locale stays LANG=C.UTF-8, re-homed to Debian with the locale data staged

**ID:** debian-staged-c-utf-8-locale
**Plan:** change-slc-runtime-debian
**Status:** Accepted
**Supersedes:** alpine-runtime-lang-c-utf-8

### Context

UDF text handling needs UTF-8 string semantics, which `C.UTF-8` provides. The image-level `ENV` does not survive tarball extraction, so the staged locale data makes the locale resolvable in the sandbox.

### Decision

The staging stage sets `ENV LANG=C.UTF-8` and stages `/usr/lib/locale/C.utf8` into the tree. No locale package is installed and no `locale-gen` runs.

### Options Considered

| Option | Verdict |
|--------|---------|
| `ENV LANG=C.UTF-8` plus staged `C.utf8` data | ✓ Chosen |
| Install `locales` and generate `en_US.UTF-8` | ✗ Adds weight for no UDF-visible benefit |

### Consequences

The staged tree carries its locale data without a `locales` package.

## ADR: Ship the "variant E" library surface and make vendoring the contract for everything else

**ID:** slc-variant-e-library-surface
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

A UDF using `native-tls` or a compression `-sys` crate fails at `dlopen` with a raw loader error if the SLC stages only the client's `ldd` closure. Staging the donor's full surface costs about 29 MB raw, against 8.2 MB raw for a curated set.

### Decision

The SLC stages OpenSSL 3 (with `ossl-modules` and `engines-3`), `zlib`, `bzip2` and `zstd` beyond the client's `ldd` closure, alongside the glibc runtime, its compatibility stubs, and the dlopen-only NSS/resolver modules. A UDF must vendor any other dynamically linked library into the `.so`, and `cargo exasol-udf validate` reports violations.

### Options Considered

| Option | Verdict |
|--------|---------|
| Curated surface (glibc, NSS/resolver, OpenSSL, zlib/bzip2/zstd) | ✓ Chosen |
| Only the client's `ldd` closure | ✗ UDFs fail at `dlopen` with no diagnosis path |
| The donor's full library surface | ✗ About 29 MB raw for an unenumerated set |

### Consequences

The library surface is a published contract with a build-time check, so authors see violations on their own machine.

## ADR: The glibc floor is 2.41, measured on this plan's own image pair, and lives in one committed file

**ID:** glibc-floor-241-single-source
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

The glibc floor is a property of the runtime distro, not the Rust toolchain. `debian:trixie-slim` ships glibc `2.41-12+deb13u3`.

### Decision

The floor is `2.41`, recorded in `crates/cargo-exasol-udf/slc-glibc-floor.txt` and read by the CLI via `include_str!`. The tarball contract test asserts that the committed value equals the highest `GLIBC_x.y` version the staged `libc.so.6` defines and that the shipped client references nothing above it.

### Options Considered

| Option | Verdict |
|--------|---------|
| One committed file, verified against the shipped `libc.so.6` | ✓ Chosen |
| An unverified inherited figure | ✗ Measured under a different variant and toolchain |
| A `const` in the CLI source | ✗ Drift checks would need to grep Rust source |
| Read the floor from the tarball at author time | ✗ Authors do not have the tarball |

### Consequences

The published floor cannot drift from what ships.

## ADR: validate errors above the floor, warns on unknown dependencies, and reads the ELF once

**ID:** validate-elf-severity-tiers
**Plan:** change-slc-runtime-debian
**Status:** Accepted

### Context

An artifact above the glibc floor cannot load, so it is an error. An artifact linking an unstaged library may still load, so failing it outright would block authors before they can vendor. Running the platform checks before `dlopen` makes them testable with a generated fixture.

### Decision

`validate` reads the ELF once, yielding the entry symbols, the `DT_NEEDED` sonames and the highest `GLIBC_x.y` reference. An artifact above the floor is a hard error. A `DT_NEEDED` entry outside the SLC surface is a warning that `--deny-unknown-deps` escalates to an error. Checks run in this order: ELF read, entry symbols, glibc floor, `DT_NEEDED`, dlopen ABI/fingerprint.

### Options Considered

| Option | Verdict |
|--------|---------|
| Error on floor, warn on unknown deps, single `goblin` read | ✓ Chosen |
| Warn on both | ✗ An artifact above the floor cannot load |
| Error on both | ✗ Blocks authors who still work today |
| Keep `nm` for symbols and parse only for new checks | ✗ Two mechanisms read the same dynamic section and require binutils |

### Consequences

`validate` runs no shell-out and needs no binutils on the author's host.
