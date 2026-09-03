# Feature: slim-image

Packages the `exaudfclient` binary into a slim, Debian-staged SLC root filesystem (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The SLC is a three-stage build from a single root `Dockerfile`. A `rust:1.94-trixie` builder compiles `exaudfclient` with zmq statically linked (no `libzmq3-dev` — `zmq-sys` falls back to `zeromq-src`). `exaudfclient` links no bzip2 at all: `exarrow-rs`'s bzip2 usage lives entirely behind its CSV `IMPORT`/`EXPORT` local-file-compression feature, a code path the client's `ExaConnection` usage never reaches, so the linker drops the dependency. The builder also derives the two architecture-dependent values — the Debian multiarch triplet and the built binary's own `PT_INTERP` loader path — and records them for the next stage, because the runtime donor image carries neither `binutils` nor `dpkg-architecture`. A `debian:trixie-slim` stage is then both donor and packager: it reproduces its own usr-merge symlink layout inside a staged `/slc` tree, copies the glibc runtime, the dlopen-only NSS/resolver modules and the documented UDF library surface out of itself with `cp -L`, adds the binary, the language-definition file and the notice bundles, and tars `/slc` into `lc-rs.tar.gz`. A final `FROM scratch` artifact stage exposes the tarball for `docker build --output`. Nothing outside that curated set ships: the staged tree carries no shell, no package manager, no coreutils, no Rust toolchain and no vendored Cargo registry, so it supports precompiled `.so` UDFs only. Every architecture-dependent path is derived rather than hardcoded, so a native build on x86_64 or aarch64 produces the matching-architecture SLC with no cross-compilation.

The staged tree is the UDF's entire root filesystem at run time, so what it provides beyond the client's own link closure is a deliberate, documented contract rather than an accident of `cp -L`. That contract — the fixed library surface and the glibc version floor it publishes to authors — is specified in `container/slc-platform-contract`, not here; this feature covers only the build mechanics that produce and package the staged tree.

The Exasol engine sets `TZ` from the session timezone for every UDF (via `NSEXEC_ENV_TZ` → `TZ`), commonly as an IANA name such as `Europe/Berlin`. The staged tree must carry the IANA zoneinfo database so `chrono::Local`/`time` resolve named zones instead of silently falling back to UTC; the runtime never reads `TZ` itself.

The SLC is distributed as a flattened root-filesystem tarball that Exasol extracts after BucketFS upload, with the executable at `/exaudf/exaudfclient`. For DNS to work inside the UDF sandbox, the tarball must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`, which the database populates at runtime. These symlinks cannot be baked as live symlinks in the image layers (`COPY` dereferences a dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths), so they are created in a staging directory and tarred inside the Docker build itself.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Language definitions file is present and well-formed

* *GIVEN* the SLC tarball, whose `build_info/language_definitions.json` the database parses and schema-validates during Engine/Nano initialization when the tarball is installed as a custom SLC
* *WHEN* `build_info/language_definitions.json` is read from it
* *THEN* it MUST declare `schema_version` `2` and MUST carry its definitions under the root key `language_definitions`, and the root key `languages` MUST NOT be present, because the parser's root-level `required` check otherwise rejects the whole file and aborts initialization into a restart loop
* *AND* it MUST hold exactly one language definition, whose `aliases` is `["RUST"]`, whose `protocol` is `localzmq+protobuf`, whose `udf_client_path.executable` is `/exaudf/exaudfclient`, whose `parameters` is an array holding the single key/value object `{"key": "lang", "value": "rust"}`, and which carries a `deprecation` key present with the value `null`
* *AND* the definition MUST NOT carry an `arguments` key, so the `lang=rust` launch parameter has exactly one representation in the file and cannot drift against `parameters`
* *AND* the definition MAY retain `language_identifier` `rust`, which this parser does not consume
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Language definition contract is asserted by key path on both copies

* *GIVEN* the repository's committed `build_info/language_definitions.json` and the copy the SLC tarball ships
* *WHEN* the language-definition contract is checked
* *THEN* each required field's absence, removal, or re-typing MUST fail the check, so a renamed root key or a re-typed field cannot pass unnoticed
* *AND* the copy the tarball ships MUST be byte-identical to the committed `build_info/language_definitions.json`, so the document that was checked is the document the database parses
* *AND* the contract MUST be checkable against the committed file alone, without building the container image, so a wrong shape fails before the build rather than after it
<!-- /DELTA:NEW -->
