# Tasks: change-docker-alpine-base

## Phase 2: Implementation (Group A — parallel Dockerfile stages)
- [x] 1.1 Add `Dockerfile.alpine` builder stage (`FROM rust:alpine`, `apk add`, workspace COPY, path rewrite)
- [x] 1.2 Add `Dockerfile.alpine` runtime stage (`FROM alpine:3`, `apk add libzmq ca-certificates`, `ENV LANG=C.UTF-8`, entrypoint)

## Phase 2: Implementation (Group B — musl build config)
- [x] 2.1 Configure Alpine builder to compile for `x86_64-unknown-linux-musl` and resolve libzmq/pkg-config cross-detection [expert]

## Phase 3: Verification (Group C — build, test, measure)
- [x] 3.1 Build Alpine image as `slc-rs-slim:dev` and run db-roundtrip integration suite; diagnose any musl-specific failures [expert]
- [x] 3.2 Measure Debian vs Alpine image sizes with `docker image inspect`, compute delta, record in plan
