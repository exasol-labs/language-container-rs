# Decision Log: change-docker-alpine-base

Date: 2026-06-08

## Interview

**Q:** What is the scope of the image spike?
**A:** Full — build plus integration tests. The Alpine image must pass the same db-roundtrip integration tests that the Bookworm image currently passes.

## Design Decisions

### [1] Add a parallel Dockerfile.alpine rather than replacing the Debian Dockerfile

- **Decision:** Introduce `Dockerfile.alpine` (builder `rust:alpine`, runtime `alpine:3`) side by side with the existing Debian `Dockerfile`; build it under the same `slc-rs-slim:dev` tag for the spike.
- **Alternatives:** Replace the Debian `Dockerfile` outright — rejected because a spike must be comparable side by side and trivially reversible if Alpine fails the suite or the size goal.
- **Rationale:** Keeps the proven Debian path intact while the Alpine path is validated; allows a direct size comparison of two images built from the same workspace.
- **Promotes to ADR:** no

### [2] Build the client binary for x86_64-unknown-linux-musl on Alpine

- **Decision:** The Alpine builder compiles `exaudfclient` for `x86_64-unknown-linux-musl` and copies the musl binary into the `alpine:3` runtime.
- **Alternatives:** Keep a glibc binary and run it on Alpine via a compatibility shim (`gcompat`) — rejected as fragile and counter to the smaller-image goal.
- **Rationale:** Alpine is musl-based; the workspace already pins and builds musl artifacts for UDF `.so` files, so the client binary aligns with established tooling.
- **Promotes to ADR:** yes

### [3] Dynamically link Alpine libzmq instead of static-linking it

- **Decision:** Install Alpine `libzmq` in the runtime and link it dynamically, mirroring the Debian image's `libzmq5` approach.
- **Alternatives:** Fully static-link `libzmq` into the musl binary — rejected as a larger, riskier change unsuitable for a spike; UDF `.so` artifacts are static but the client binary need not be.
- **Rationale:** Reuses the proven dynamic-link shape; isolates the spike's variable to base distro + libc rather than also changing the linking model.
- **Promotes to ADR:** no

### [4] Locale via LANG=C.UTF-8, no locale-gen

- **Decision:** Set `ENV LANG=C.UTF-8` in the Alpine runtime and install no locale package.
- **Alternatives:** Install `musl-locales` and generate `en_US.UTF-8` to match the Debian image — rejected as unnecessary weight; `C.UTF-8` is the musl default and sufficient for UDF text handling.
- **Rationale:** Alpine/musl ships no `locales` package; `locale-gen` does not exist there. `C.UTF-8` keeps the image minimal while preserving UTF-8 behavior.
- **Promotes to ADR:** yes

### [5] Validate the spike through the existing db-roundtrip suite under the slc-rs-slim:dev tag

- **Decision:** Prove the Alpine image by building it as `slc-rs-slim:dev` and running the existing `db_roundtrip_all_scenarios` integration test, which exports that tag into BucketFS — no new integration test harness is added.
- **Alternatives:** Write a separate Alpine-only integration test — rejected; the harness keys off the `SLC_IMAGE` tag, so reusing it gives identical coverage with zero new test surface.
- **Rationale:** Interchangeability with the Debian image is exactly what the spike must prove; reusing the suite makes the comparison apples-to-apples.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
