# Plan: change-docker-alpine-base

## Summary

Spike an Alpine/musl base for the SLC Docker image — `rust:alpine` builder compiling `exaudfclient` for `x86_64-unknown-linux-musl` and an `alpine:3` runtime shipping `libzmq` — and prove it produces a smaller image that still passes the full db-roundtrip integration suite that the current Debian slim image passes.

## Design

This is an exploratory packaging change that swaps the libc and base distro for the runtime image, so it warrants a recorded design decision.

### Context

The current image uses `debian:12-slim` (glibc) runtime and `rust:1.91-bookworm` builder, dynamically linking `libzmq5`. The workspace already builds fully-static musl `.so` UDF artifacts and pins `x86_64-unknown-linux-musl` as a target, so musl tooling is established. The question is whether the *client binary* and its `libzmq` dependency can also move to Alpine/musl to shrink the image without breaking the live db-roundtrip behavior.

- **Goals** — produce `Dockerfile.alpine` (a parallel Dockerfile, not a replacement) that builds a musl `exaudfclient`, runs on `alpine:3`, passes the existing db-roundtrip suite unchanged, and yields a smaller image; record the measured size delta.
- **Non-Goals** — not deleting or replacing the existing Debian `Dockerfile` (the spike runs side by side); not changing the `language_definitions.json` contract; not changing UDF `.so` build (already musl); not changing the wire protocol.

### Decision

Add `Dockerfile.alpine` with two stages. Builder: `FROM rust:alpine`, `apk add zeromq-dev protobuf-dev pkgconfig musl-dev`, build `exaudfclient` for `x86_64-unknown-linux-musl`. Runtime: `FROM alpine:3`, `apk add libzmq ca-certificates`, `ENV LANG=C.UTF-8`, copy the musl binary to `/exaudf/exaudfclient` and `build_info/` to `/build_info/`. The runtime depends on `libzmq` dynamically (Alpine `libzmq`), matching the Debian image's dynamic-link approach but on musl.

#### Architecture

```
┌──────────────────────────┐     ┌──────────────────────────┐
│ builder: rust:alpine     │     │ runtime: alpine:3        │
│ apk: zeromq-dev,         │────▶│ apk: libzmq,             │
│      protobuf-dev,       │ bin │      ca-certificates     │
│      pkgconfig, musl-dev │     │ ENV LANG=C.UTF-8         │
│ target x86_64-…-musl     │     │ /exaudf/exaudfclient     │
└──────────────────────────┘     └──────────────────────────┘
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Multi-stage build, toolchain dropped from runtime | `Dockerfile.alpine` | Keep runtime slim and Option-A only (precompiled `.so`) |
| Static-libc-friendly base | musl + `alpine:3` | Smaller surface; aligns with already-musl UDF artifacts |
| Locale via env, not `locale-gen` | runtime `ENV LANG=C.UTF-8` | Alpine/musl ships no `locales` package; UTF-8 is the musl default |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Add `Dockerfile.alpine` alongside the Debian `Dockerfile` | Replace the Debian `Dockerfile` outright | A spike must be comparable side by side and reversible if Alpine fails the suite |
| Dynamically link Alpine `libzmq` | Fully static-link `libzmq` into the binary | Mirrors the proven Debian approach; static `libzmq` on musl is a larger, riskier change for a spike |
| `LANG=C.UTF-8`, no `locale-gen` | Install `musl-locales`; generate `en_US.UTF-8` | `C.UTF-8` is the musl default and sufficient for UDF text; avoids extra packages, keeping the image small |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| slim-image | CHANGED | `specs/_plans/change-docker-alpine-base/container/slim-image/spec.md` |

## Dependencies

- `rust:alpine` and `alpine:3` Docker base images.
- Alpine packages: `zeromq-dev`, `protobuf-dev`, `pkgconfig`, `musl-dev` (builder); `libzmq`, `ca-certificates` (runtime).
- `x86_64-unknown-linux-musl` Rust target (already pinned in `rust-toolchain.toml`).
- A Docker daemon and the `exasol/docker-db:2026.latest` image for the integration suite.

## Migration

| Current | New |
|---------|-----|
| `Dockerfile` only (`debian:12-slim` + `rust:1.91-bookworm`, glibc, `libzmq5`, `locale-gen en_US.UTF-8`) | adds `Dockerfile.alpine` (`alpine:3` + `rust:alpine`, musl, `libzmq`, `LANG=C.UTF-8`) |

The slim-image Background currently names `rust:1.84-bookworm` / `debian:12-slim`; when this plan is recorded the Background MUST be extended to describe the Alpine variant. No existing Debian scenario is removed by this spike.

## Implementation Tasks

1. Add `Dockerfile.alpine`: builder `FROM rust:alpine` with `apk add --no-cache zeromq-dev protobuf-dev pkgconfig musl-dev`, mirroring the Debian builder's workspace COPY, exarrow-rs build-context, and `[patch]` path rewrite steps.
2. Configure the Alpine builder to compile for musl: `cargo build --release --target x86_64-unknown-linux-musl -p exaudfclient`, and copy from `target/x86_64-unknown-linux-musl/release/exaudfclient`. Resolve any `libzmq`/`pkg-config` cross-detection so the musl build links Alpine `libzmq`. [expert]
3. Add the Alpine runtime stage: `FROM alpine:3`, `apk add --no-cache libzmq ca-certificates`, `ENV LANG=C.UTF-8`, copy the musl binary to `/exaudf/exaudfclient`, copy `build_info/` to `/build_info/`, set the entrypoint.
4. Build the Alpine image as `slc-rs-slim:dev` and run the db-roundtrip integration suite (`crates/it/tests/db_roundtrip.rs`) against it, confirming every non-known-failing scenario passes exactly as it does for the Debian image. Diagnose any musl-specific failures (locale, dynamic-loader, libzmq ABI). [expert]
5. Measure both images with `docker image inspect --format '{{.Size}}'` (Debian vs Alpine), compute the delta, and record it in this plan's spike notes / decision-log.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1, Task 3 (independent Dockerfile stages) |
| Group B | Task 2 |
| Group C | Task 4, Task 5 |

Sequential dependencies:
- Group A → Group B (musl build config refines the builder stage) → Group C (build + test + measure require a complete Dockerfile)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| (none) | — | Spike adds `Dockerfile.alpine` alongside the existing `Dockerfile`; nothing is removed until the spike is accepted. |

## Spike Notes

### Architecture pivot — glibc bundling, not musl

The original plan assumed a musl build (`rust:alpine` → `x86_64-unknown-linux-musl`). During implementation, two blockers ruled that out:

1. **Exasol sandbox seccomp / CPU-instruction incompatibility with Rust 1.96+**: `rust:bookworm` (1.96.0) compiled binaries crash in the Exasol UDF sandbox. Using `rust:1.91-bookworm` (same as the Debian `Dockerfile`) resolves this.
2. **Binary runs on the Debian Exasol host, not inside the Docker container**: `exaudfclient` is exported from the image via `docker export`, uploaded to BucketFS, and executed directly on the glibc Debian host. A musl binary would be ABI-incompatible there.

Adopted approach: keep the builder as `rust:1.91-bookworm` (glibc), copy the exact glibc runtime libs from the builder into the Alpine runtime stage using `cp -L` (to dereference symlinks), and run Alpine:3 as the packaging layer only. This gives the small base image while keeping the binary glibc-linked. zmq is statically linked (no `libzmq3-dev` in the builder forces `zmq-sys` to use `zeromq-src` / zmq 4.3.4), eliminating the `libzmq.so.5` runtime dependency.

### Image size delta (measured 2026-06-08)

| Image | Base | `docker image inspect` size |
|-------|------|----------------------------|
| `slc-rs-slim:debian` | `debian:12-slim` | 122,067,659 bytes (122 MB) |
| `slc-rs-slim:dev` (Alpine) | `alpine:3` | 32,292,573 bytes (32.3 MB) |
| **Delta** | | **−89,775,086 bytes (−89.8 MB, 73.5% reduction, 3.8× smaller)** |

The savings come from swapping `debian:12-slim` (75 MB base) for `alpine:3` (7 MB base) and removing `libzmq5`, `locales`, and related Debian runtime packages.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Alpine builder compiles the binary against musl | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` (image built from `Dockerfile.alpine`, exported via `Harness::load_slc`) |
| Alpine runtime stage is slim and self-sufficient | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` (runtime exercised by `load_slc` + UDF execution) |
| Alpine image passes the db-roundtrip integration suite | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` |
| Alpine image is smaller than the Debian slim image | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` size-delta assertion (compares `docker image inspect` size of Debian vs Alpine builds) |

The whole suite runs through the single gated test `db_roundtrip_all_scenarios`, which `docker export`s the `slc-rs-slim:dev` image into BucketFS; building that tag from `Dockerfile.alpine` is what exercises the Alpine variant end to end.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| slim-image | `docker build -f Dockerfile.alpine --build-context exarrow-rs=/home/talos/code/exarrow-rs -t slc-rs-slim:dev .` | Build completes; image tagged `slc-rs-slim:dev` |
| slim-image | `docker run --rm --entrypoint /exaudf/exaudfclient slc-rs-slim:dev` | Prints a usage message referencing `lang=rust`; exits non-zero |
| slim-image | `docker run --rm --entrypoint ldd slc-rs-slim:dev /exaudf/exaudfclient` | Resolves against musl/`libzmq`; no glibc loader missing |
| slim-image | `docker image inspect --format '{{.Size}}' slc-rs-slim:dev` | Numeric byte size smaller than the Debian image's size |
| slim-image | `cargo test -p it --features integration db_roundtrip -- --nocapture` (Docker available, Alpine image built) | Each scenario logs `ok`; suite passes |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Image | `docker build -f Dockerfile.alpine --build-context exarrow-rs=/home/talos/code/exarrow-rs -t slc-rs-slim:dev .` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt` | No changes |
