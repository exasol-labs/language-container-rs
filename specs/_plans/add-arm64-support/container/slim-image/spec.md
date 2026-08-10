# Feature: slim-image

Packages the `exaudfclient` binary into a slim Alpine-based SLC Docker image (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

<!-- DELTA:CHANGED -->
The slim image is a multi-stage build: a `rust:1.94-bookworm` builder compiles `exaudfclient` with zmq statically linked (no `libzmq3-dev` — `zmq-sys` falls back to `zeromq-src`), then copies the glibc runtime libs (`libc.so.6`, `libm.so.6`, `libgcc_s.so.1`, `libstdc++.so.6`, the dynamic loader, NSS modules) with `cp -L` into an `alpine:3` runtime stage. The build is architecture-neutral: the Debian multiarch triplet and the loader path are derived from the build host rather than hardcoded, so a native build on x86_64 or aarch64 produces the matching-architecture container with no cross-compilation. The runtime stage ships only `ca-certificates` and the bundled glibc, placing the binary at `/exaudf/exaudfclient` and the language registration file at `/build_info/language_definitions.json`. The image carries no Rust toolchain and no vendored registry, so it supports precompiled `.so` UDFs only. The `exaudfclient` binary is glibc-linked — it runs on the Debian/glibc Exasol host after BucketFS extraction; Alpine serves as the packaging layer only.
<!-- /DELTA:CHANGED -->

The Exasol engine sets `TZ` from the session timezone for every UDF (via `NSEXEC_ENV_TZ` → `TZ`), commonly as an IANA name such as `Europe/Berlin`. The runtime image must bundle the IANA zoneinfo database so `chrono::Local`/`time` resolve named zones instead of silently falling back to UTC; the runtime never reads `TZ` itself.

The SLC is distributed as a flattened root-filesystem tarball that Exasol extracts after BucketFS upload, with the executable at `/exaudf/exaudfclient`. For DNS to work inside the UDF sandbox, the tarball must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`, which the database populates at runtime. These symlinks cannot be baked as live symlinks in the image layers (`COPY` dereferences a dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths), so they are created in a staging directory and tarred inside the Docker build itself.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Builder toolchain and glibc runtime

* *GIVEN* the Dockerfile builder stage `FROM rust:1.94-bookworm`
* *WHEN* the image is built
* *THEN* the builder MUST install `protobuf-compiler` and `pkg-config` but NOT `libzmq3-dev`, and zmq MUST be statically linked via `zeromq-src`
* *AND* the glibc runtime libs MUST be collected via `cp -L` into `/glibc-rt/` under the build architecture's Debian multiarch triplet, derived at build time rather than hardcoded to `x86_64-linux-gnu`, with the dynamic loader staged at exactly the built binary's own `PT_INTERP` path — a loader staged elsewhere leaves `PT_INTERP` dangling and every UDF dies as a bare `22002 VM crashed`
* *AND* the builder image tag MUST match the channel pinned in `rust-toolchain.toml` (`1.94`); the spec MUST NOT name a stale builder tag that no longer matches the toolchain pin
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: SLC builds natively for the host architecture

* *GIVEN* a build host of architecture `x86_64` or `aarch64` with Docker
* *WHEN* `docker build -f Dockerfile.alpine --target artifact` runs natively, without QEMU emulation or cross-compilation
* *THEN* the produced `/exaudf/exaudfclient` MUST be an ELF binary of the build host's architecture, with its bundled glibc loader at the binary's own `PT_INTERP` path so the UDF sandbox resolves its interpreter
* *AND* an empty derived multiarch triplet or loader path MUST fail the build with a named error rather than a cryptic downstream `cp` failure
* *AND* the resulting SLC MUST be registrable and execute UDFs on an Exasol host of that architecture, with no change to the `language_definitions.json` contract
<!-- /DELTA:NEW -->
