# Feature: slim-image

Packages the `exaudfclient` binary into a slim Debian-based SLC Docker image (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The slim image is a multi-stage build: a `rust:1.84-bookworm` builder compiles `exaudfclient` against `libzmq3-dev` and `protobuf-compiler`, and a `debian:12-slim` runtime stage ships only `libzmq5`, `ca-certificates`, and a UTF-8 locale. The runtime stage places the binary at `/exaudf/exaudfclient` and the language registration file at `/build_info/language_definitions.json`. The image carries no Rust toolchain and no vendored registry, so it supports precompiled `.so` UDFs only. The builder's toolchain MUST match `rust-toolchain.toml` (`1.84`) so Option-A artifacts pass the fingerprint check.

<!-- NEW -->

## Scenarios

### Scenario: docker build produces a tagged slim image

* *GIVEN* the workspace with a `Dockerfile` at the repository root
* *WHEN* `docker build -t slc-rs-slim:dev .` is run
* *THEN* the build MUST complete successfully
* *AND* the resulting image MUST contain an executable at `/exaudf/exaudfclient`

### Scenario: Builder toolchain matches the pinned channel

* *GIVEN* the Dockerfile builder stage `FROM rust:1.84-bookworm`
* *WHEN* the image is built
* *THEN* the builder Rust version MUST equal the channel pinned in `rust-toolchain.toml`
* *AND* the builder stage MUST install `libzmq3-dev`, `protobuf-compiler`, and `pkg-config`

### Scenario: Runtime stage is slim and self-sufficient

* *GIVEN* the Dockerfile runtime stage `FROM debian:12-slim`
* *WHEN* the image is built
* *THEN* it MUST install `libzmq5`, `ca-certificates`, and `locales` with `en_US.UTF-8` generated and `LANG=en_US.UTF-8` set
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

<!-- /NEW -->
