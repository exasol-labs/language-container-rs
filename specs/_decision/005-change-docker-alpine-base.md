# Decisions: change-docker-alpine-base

## ADR: Alpine image — build the client binary for x86_64-unknown-linux-musl

**ID:** alpine-image-musl-client-binary
**Plan:** `change-docker-alpine-base`
**Status:** Accepted

### Context

The original Alpine image design compiled `exaudfclient` for the musl target (`x86_64-unknown-linux-musl`) using a `rust:alpine` builder, aligning with the already-musl UDF `.so` artifacts. During implementation, two blockers ruled this out: Rust 1.96+ compiled binaries crashed in the Exasol UDF sandbox due to seccomp/CPU-instruction incompatibility, and the `exaudfclient` binary is executed directly on the glibc Debian Exasol host after BucketFS extraction — a musl binary would be ABI-incompatible there. The adopted approach bundled glibc runtime libs into the Alpine image instead. The decision entry records what was resolved at plan time; the implementation pivot is documented in the plan's spike notes.

### Decision

The Alpine builder stage compiles `exaudfclient` for `x86_64-unknown-linux-musl` on a `rust:alpine` builder, and the resulting musl binary is placed in the `alpine:3` runtime stage.

### Options Considered

| Option | Verdict |
|--------|---------|
| Compile for `x86_64-unknown-linux-musl` on `rust:alpine` | ✓ Chosen — aligns with already-musl UDF artifacts; Alpine is musl-based; no glibc compat shim needed |
| Keep a glibc binary and run it on Alpine via `gcompat` | ✗ Rejected — fragile and counter to the smaller-image goal |

### Consequences

The Alpine builder must install `zeromq-dev`, `protobuf-dev`, `pkgconfig`, and `musl-dev` via `apk`. The runtime binary requires no glibc loader on `alpine:3`. See plan spike notes for the implementation pivot to glibc-bundling that superseded this in practice.

## ADR: Alpine runtime uses LANG=C.UTF-8 instead of locale-gen

**ID:** alpine-runtime-lang-c-utf-8
**Plan:** `change-docker-alpine-base`
**Status:** Accepted

### Context

The Debian slim image runs `locale-gen en_US.UTF-8` to configure the locale. Alpine/musl ships no `locales` package and no `locale-gen` binary. A decision was needed on how to configure UTF-8 locale in the Alpine runtime stage.

### Decision

Set `ENV LANG=C.UTF-8` in the Alpine runtime stage. No locale package is installed; no `locale-gen` is run.

### Options Considered

| Option | Verdict |
|--------|---------|
| `ENV LANG=C.UTF-8`, no locale package | ✓ Chosen — `C.UTF-8` is the musl default and sufficient for UDF text handling; keeps the image minimal; `locale-gen` does not exist on Alpine |
| Install `musl-locales` and generate `en_US.UTF-8` | ✗ Rejected — unnecessary weight; matches the Debian convention but adds extra packages without benefit |

### Consequences

The Alpine runtime carries no locale package. `C.UTF-8` provides UTF-8 string semantics adequate for UDF text handling. The absence of `locale-gen` is a non-issue on Alpine/musl. The runtime stage installs only `ca-certificates` via `apk`.
