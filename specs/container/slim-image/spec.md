# Feature: slim-image

Packages the `exaudfclient` binary into a slim Alpine-based SLC Docker image (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The slim image is a multi-stage build: a `rust:1.94-bookworm` builder compiles `exaudfclient` with zmq statically linked (no `libzmq3-dev` — `zmq-sys` falls back to `zeromq-src`), then copies the glibc runtime libs (`libc.so.6`, `libm.so.6`, `libgcc_s.so.1`, `libstdc++.so.6`, `ld-linux-x86-64.so.2`, NSS modules) with `cp -L` into an `alpine:3` runtime stage. The runtime stage ships only `ca-certificates` and the bundled glibc, placing the binary at `/exaudf/exaudfclient` and the language registration file at `/build_info/language_definitions.json`. The image carries no Rust toolchain and no vendored registry, so it supports precompiled `.so` UDFs only. The `exaudfclient` binary is glibc-linked — it runs on the Debian/glibc Exasol host after BucketFS extraction; Alpine serves as the packaging layer only.

The Exasol engine sets `TZ` from the session timezone for every UDF (via `NSEXEC_ENV_TZ` → `TZ`), commonly as an IANA name such as `Europe/Berlin`. The runtime image must bundle the IANA zoneinfo database so `chrono::Local`/`time` resolve named zones instead of silently falling back to UTC; the runtime never reads `TZ` itself.

The SLC is distributed as a flattened root-filesystem tarball that Exasol extracts after BucketFS upload, with the executable at `/exaudf/exaudfclient`. For DNS to work inside the UDF sandbox, the tarball must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`, which the database populates at runtime. These symlinks cannot be baked as live symlinks in the image layers (`COPY` dereferences a dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths), so they are created in a staging directory and tarred inside the Docker build itself.

## Scenarios

### Scenario: docker build produces a tagged slim image

* *GIVEN* the workspace with a `Dockerfile.alpine` at the repository root
* *WHEN* `docker build -f Dockerfile.alpine -t lc-rs-slim:dev .` is run
* *THEN* the build MUST complete successfully
* *AND* the resulting image MUST contain an executable at `/exaudf/exaudfclient`

### Scenario: Builder toolchain and glibc runtime

* *GIVEN* the Dockerfile builder stage `FROM rust:1.94-bookworm`
* *WHEN* the image is built
* *THEN* the builder MUST install `protobuf-compiler` and `pkg-config` but NOT `libzmq3-dev`
* *AND* zmq MUST be statically linked via `zeromq-src`
* *AND* the glibc runtime libs MUST be collected via `cp -L` into `/glibc-rt/` and staged into the runtime image
* *AND* the builder image tag MUST match the channel pinned in `rust-toolchain.toml` (`1.94`); the spec MUST NOT name a stale builder tag that no longer matches the toolchain pin

### Scenario: Runtime stage is slim and self-sufficient

* *GIVEN* the Dockerfile runtime stage `FROM alpine:3`
* *WHEN* the image is built
* *THEN* it MUST install only `ca-certificates` via `apk`
* *AND* it MUST set `ENV LANG=C.UTF-8`
* *AND* it MUST NOT contain a Rust toolchain or a vendored Cargo registry

### Scenario: Language definitions file is present and well-formed

* *GIVEN* the runtime image
* *WHEN* `/build_info/language_definitions.json` is read
* *THEN* it MUST declare `schema_version` `2`
* *AND* it MUST contain one language definition with protocol `localzmq+protobuf`, alias `RUST`, parameter `lang=rust`, and `udf_client_path.executable` equal to `/exaudf/exaudfclient`

### Scenario: Binary runs and reports its usage in the image

* *GIVEN* the built slim image
* *WHEN* `/exaudf/exaudfclient` is invoked with no arguments inside the container
* *THEN* it MUST print a usage message referencing `lang=rust`
* *AND* it MUST exit with a non-zero code

### Scenario: Alpine runtime stage is slim and self-sufficient

* *GIVEN* the `Dockerfile.alpine` runtime stage `FROM alpine:3`
* *WHEN* the image is built
* *THEN* it MUST install `libzmq`, `ca-certificates`, and `tzdata` via `apk`
* *AND* installing `tzdata` MUST populate the IANA zoneinfo database at `/usr/share/zoneinfo` so a DB-supplied named `TZ` resolves to a real zone instead of UTC
* *AND* it MUST set `LANG=C.UTF-8` rather than running `locale-gen`, because Alpine/musl provides no `locales` package
* *AND* it MUST NOT contain a Rust toolchain or a vendored Cargo registry

### Scenario: Alpine image passes the db-roundtrip integration suite

* *GIVEN* the `lc-rs-slim:dev` image built from `Dockerfile.alpine` and a running `exasol/docker-db:2026.latest` container
* *WHEN* the db-roundtrip integration harness registers the Alpine SLC, uploads the UDF artifacts, and runs every roundtrip scenario
* *THEN* the scalar, set/EMITS, statically-linked-dependency, UDF-error, and single-call scenarios MUST all pass against the Alpine image
* *AND* the Alpine image MUST be interchangeable with the Debian image for SLC registration, requiring no change to the `language_definitions.json` contract

### Scenario: Alpine image is smaller than the Debian slim image

* *GIVEN* both the Debian `lc-rs-slim:dev` image and the Alpine `lc-rs-slim:dev` image built from the same workspace
* *WHEN* the compressed and on-disk sizes of both images are measured with `docker image inspect`
* *THEN* the Alpine image on-disk size MUST be smaller than the Debian slim image
* *AND* the measured size delta MUST be recorded in the plan's spike notes

### Scenario: SLC tarball ships the /conf resolver symlinks

* *GIVEN* the SLC distribution tarball produced from `Dockerfile.alpine` by the Docker build alone, without any host-side post-processing step
* *WHEN* the entries for `etc/hosts` and `etc/resolv.conf` are inspected
* *THEN* `etc/hosts` MUST be a symbolic-link entry pointing to `/conf/hosts`
* *AND* `etc/resolv.conf` MUST be a symbolic-link entry pointing to `/conf/resolv.conf`
* *AND* producing the tarball MUST NOT require any interpreter or tool outside the Docker build environment (no host `python3`)

### Scenario: Runtime image bundles the IANA zoneinfo database

* *GIVEN* the built slim Alpine image and that the database always sends the session timezone as `TZ` for every UDF
* *WHEN* the runtime filesystem is inspected for the zoneinfo database
* *THEN* `/usr/share/zoneinfo/Europe/Berlin` MUST exist as a readable zone file
* *AND* the fix MUST be packaging only (an `apk add` of `tzdata`), since `chrono`/`time` consult the zoneinfo database implicitly and the runtime MUST NOT read `TZ` itself
