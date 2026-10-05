# Decisions: change-docker-alpine-base

## ADR: Alpine image: build the client binary for x86_64-unknown-linux-musl

**ID:** alpine-image-musl-client-binary
**Plan:** `change-docker-alpine-base`
**Status:** Superseded by debian-trixie-slim-staged-runtime

### Context

Alpine is musl-based, and the UDF `.so` artifacts already target musl.

### Decision

The Alpine builder stage compiles `exaudfclient` for `x86_64-unknown-linux-musl` on a `rust:alpine` builder, and the musl binary is placed in the `alpine:3` runtime stage.

### Options Considered

| Option | Verdict |
|--------|---------|
| Compile for `x86_64-unknown-linux-musl` on `rust:alpine` | ✓ Chosen, no glibc shim |
| Glibc binary on Alpine via `gcompat` | ✗ Fragile, defeats the smaller-image goal |

### Consequences

The Alpine builder installs `zeromq-dev`, `protobuf-dev`, `pkgconfig` and `musl-dev` via `apk`. The runtime binary needs no glibc loader.

## ADR: Alpine runtime uses LANG=C.UTF-8 instead of locale-gen

**ID:** alpine-runtime-lang-c-utf-8
**Plan:** `change-docker-alpine-base`
**Status:** Superseded by debian-staged-c-utf-8-locale

### Context

Alpine/musl ships no `locales` package and no `locale-gen` binary, so the Debian `locale-gen en_US.UTF-8` step does not apply.

### Decision

The Alpine runtime stage sets `ENV LANG=C.UTF-8`. It installs no locale package and runs no `locale-gen`.

### Options Considered

| Option | Verdict |
|--------|---------|
| `ENV LANG=C.UTF-8` | ✓ Chosen, musl default, sufficient for UDF text |
| Install `musl-locales`, generate `en_US.UTF-8` | ✗ Extra packages without benefit |

### Consequences

The runtime stage installs only `ca-certificates` via `apk`.
