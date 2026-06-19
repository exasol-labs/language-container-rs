# Feature: slim-image

Packages the `exaudfclient` binary into a slim Alpine-based SLC Docker image (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The Exasol engine sets `TZ` from the session timezone for every UDF (via `NSEXEC_ENV_TZ` → `TZ`), commonly as an IANA name such as `Europe/Berlin`. The runtime image must bundle the IANA zoneinfo database so `chrono::Local`/`time` resolve named zones instead of silently falling back to UTC; the runtime never reads `TZ` itself.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Alpine runtime stage is slim and self-sufficient

* *GIVEN* the `Dockerfile.alpine` runtime stage `FROM alpine:3`
* *WHEN* the image is built
* *THEN* it MUST install `libzmq`, `ca-certificates`, and `tzdata` via `apk`
* *AND* installing `tzdata` MUST populate the IANA zoneinfo database at `/usr/share/zoneinfo` so a DB-supplied named `TZ` resolves to a real zone instead of UTC
* *AND* it MUST set `LANG=C.UTF-8` rather than running `locale-gen`, because Alpine/musl provides no `locales` package
* *AND* it MUST NOT contain a Rust toolchain or a vendored Cargo registry
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Runtime image bundles the IANA zoneinfo database

* *GIVEN* the built slim Alpine image and that the database always sends the session timezone as `TZ` for every UDF
* *WHEN* the runtime filesystem is inspected for the zoneinfo database
* *THEN* `/usr/share/zoneinfo/Europe/Berlin` MUST exist as a readable zone file
* *AND* the fix MUST be packaging only (an `apk add` of `tzdata`), since `chrono`/`time` consult the zoneinfo database implicitly and the runtime MUST NOT read `TZ` itself
<!-- /DELTA:NEW -->
