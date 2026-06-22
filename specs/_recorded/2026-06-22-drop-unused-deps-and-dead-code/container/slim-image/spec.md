# Feature: slim-image

Packages the `exaudfclient` binary into a slim Alpine-based SLC Docker image (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The slim image is a multi-stage build whose builder stage MUST use the same Rust channel pinned in `rust-toolchain.toml`. This delta reconciles two stale items: the builder tag named `rust:1.91-bookworm` while the pin is `1.92`, and a stale scenario describing a musl build that contradicts the actual glibc-linked `Dockerfile.alpine` (which bundles glibc and runs on the Debian/glibc Exasol host; Alpine is the packaging layer only).

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Builder toolchain and glibc runtime

* *GIVEN* the Dockerfile builder stage `FROM rust:1.92-bookworm`
* *WHEN* the image is built
* *THEN* the builder MUST install `protobuf-compiler` and `pkg-config` but NOT `libzmq3-dev`
* *AND* zmq MUST be statically linked via `zeromq-src`
* *AND* the glibc runtime libs MUST be collected via `cp -L` into `/glibc-rt/` and staged into the runtime image
* *AND* the builder image tag MUST match the channel pinned in `rust-toolchain.toml` (`1.92`); the spec MUST NOT name a stale `1.91` builder that no longer matches the toolchain pin
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Alpine builder compiles the binary against musl

* *GIVEN* a `Dockerfile.alpine` whose builder stage is `FROM rust:alpine`
* *WHEN* the image is built
* *THEN* the builder stage MUST install `zeromq-dev`, `protobuf-dev`, and `pkgconfig` via `apk`
* *AND* it MUST compile `exaudfclient` for the `x86_64-unknown-linux-musl` target
* *AND* the resulting binary MUST be a musl binary that runs on an `alpine:3` runtime without a glibc loader
<!-- /DELTA:REMOVED -->
