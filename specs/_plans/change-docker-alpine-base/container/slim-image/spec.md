# Feature: slim-image

Packages the `exaudfclient` binary into a slim SLC Docker image (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The slim image is a multi-stage build that ships only the `exaudfclient` binary, the dynamic `libzmq` it links against, `ca-certificates`, and a UTF-8 locale. The runtime stage places the binary at `/exaudf/exaudfclient` and the language registration file at `/build_info/language_definitions.json`. The image carries no Rust toolchain and no vendored registry, so it supports precompiled `.so` UDFs only.

<!-- DELTA:NEW -->
An Alpine/musl variant is provided via `Dockerfile.alpine`: a `rust:alpine` builder compiles `exaudfclient` for `x86_64-unknown-linux-musl`, and an `alpine:3` runtime ships `libzmq` and `ca-certificates` with `LANG=C.UTF-8` (no `locale-gen`, since Alpine/musl provides no `locales` package). The Alpine image is built under the same `slc-rs-slim:dev` tag and is interchangeable with the Debian image for SLC registration.
<!-- /DELTA:NEW -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Alpine builder compiles the binary against musl

* *GIVEN* a `Dockerfile.alpine` whose builder stage is `FROM rust:alpine`
* *WHEN* the image is built
* *THEN* the builder stage MUST install `zeromq-dev`, `protobuf-dev`, and `pkgconfig` via `apk`
* *AND* it MUST compile `exaudfclient` for the `x86_64-unknown-linux-musl` target
* *AND* the resulting binary MUST be a musl binary that runs on an `alpine:3` runtime without a glibc loader
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Alpine runtime stage is slim and self-sufficient

* *GIVEN* the `Dockerfile.alpine` runtime stage `FROM alpine:3`
* *WHEN* the image is built
* *THEN* it MUST install `libzmq` and `ca-certificates` via `apk`
* *AND* it MUST set `LANG=C.UTF-8` rather than running `locale-gen`, because Alpine/musl provides no `locales` package
* *AND* it MUST place the binary at `/exaudf/exaudfclient` and the language registration file at `/build_info/language_definitions.json`
* *AND* it MUST NOT contain a Rust toolchain or a vendored Cargo registry
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Alpine image passes the db-roundtrip integration suite

* *GIVEN* the `slc-rs-slim:dev` image built from `Dockerfile.alpine` and a running `exasol/docker-db:2026.latest` container
* *WHEN* the db-roundtrip integration harness registers the Alpine SLC, uploads the UDF artifacts, and runs every roundtrip scenario
* *THEN* the scalar, set/EMITS, statically-linked-dependency, UDF-error, and single-call scenarios MUST all pass against the Alpine image
* *AND* the Alpine image MUST be interchangeable with the Debian image for SLC registration, requiring no change to the `language_definitions.json` contract
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Alpine image is smaller than the Debian slim image

* *GIVEN* both the Debian `slc-rs-slim:dev` image and the Alpine `slc-rs-slim:dev` image built from the same workspace
* *WHEN* the compressed and on-disk sizes of both images are measured with `docker image inspect`
* *THEN* the Alpine image on-disk size MUST be smaller than the Debian slim image
* *AND* the measured size delta MUST be recorded in the plan's spike notes
<!-- /DELTA:NEW -->
