# Tasks: add-dns-name-resolution

## Phase 2: Implementation (Group A — parallel)

- [x] 1.1 Dockerfile.alpine: add `packager` stage FROM runtime that copies rootfs into staging dir (excluding ./slc, ./etc/hosts, ./etc/resolv.conf, ./proc, ./sys, ./dev), creates ln -sf symlinks, and tars to /slc-rs.tar.gz
- [x] 1.2 Dockerfile.alpine: add `artifact` stage FROM scratch that COPY --from=packager /slc-rs.tar.gz /
- [x] 2.1 Create test-udfs/resolv-udf/Cargo.toml (cdylib, deps on exasol-udf-sdk + exasol-udf-macros) and add the crate to workspace members in Cargo.toml
- [x] 2.2 Implement resolv_udf in test-udfs/resolv-udf/src/lib.rs: read col 0 as string host, resolve via std::net::ToSocketAddrs, emit first IP as Value::String, UdfError on failure or wrong type
- [x] 2.3 Add unit tests for resolv_udf: localhost resolves to IP, unresolvable host yields UdfError, non-string input yields UdfError
- [x] 8.1 Bump workspace version in Cargo.toml from 0.8.0 to 0.9.0

## Phase 2: Implementation (Group B — after Group A)

- [x] 3.1 Rewrite crates/it/src/lib.rs load_slc() to read the file at SLC_TARBALL and upload its bytes; return clear error if SLC_TARBALL unset
- [x] 3.2 Delete export_image_filesystem() and SLC_IMAGE constant from crates/it/src/lib.rs; remove dead docker create/export imports/usages
- [x] 3.3 Add resolv-udf upload + SCALAR resolv_udf script registration to crates/it tests, and add DNS-gate roundtrip test (result parses as IpAddr) + unresolvable-host negative test
- [x] 4.1 scripts/install.sh: replace docker create/export|gzip block with docker build --target artifact --output; upload slc-rs.tar.gz
- [x] 5.1 scripts/ci-it-local.sh: replace docker build -t slc-rs-slim:dev with --target artifact --output; export SLC_TARBALL
- [x] 6.1 .github/workflows/ci.yml build-slc job: replace docker save with docker build --target artifact --output producing slc-rs.tar.gz, upload as slc-tarball artifact
- [x] 6.2 .github/workflows/ci.yml integration job: download slc-tarball artifact and set SLC_TARBALL; drop docker load -i slc-image.tar step
- [x] 6.3 .github/workflows/ci.yml release job: download slc-tarball and publish directly; drop docker load + docker export|gzip block
- [x] 7.1 Confirm scripts/patch-slc-symlinks.py does not exist; if present, delete it

## Phase 3: Verification

- [x] V.1 cargo build --release (exit 0) — PASSED
- [x] V.2 cargo test (0 failures) — PASSED (2 pre-existing dispatch.rs failures confirmed on main)
- [x] V.3 cargo fmt --check && cargo clippy --exclude it --workspace -- -D warnings — PASSED
- [x] V.4 Live integration (Exasol 2025.1.11): all 13 scenarios pass incl. resolv_udf DNS gate — proves the packager symlinks resolve DNS inside the UDF sandbox end-to-end

## Code Review Fixes (post-review)

- [x] CR.1 Added -p resolv-udf to ci.yml build job
- [x] CR.2 Added -p resolv-udf to ci-it-local.sh build list
- [x] CR.3 Fixed stale install.sh header comment
- [x] CR.4 Fixed stale ci-it-local.sh SKIP_SLC_BUILD doc comment
- [x] CR.5 Fixed ci-it-local.sh teardown bug: mid-script cleanup() deleted the SLC_DIR tarball before tests ran; split into cleanup() (container only) + on_exit() (also removes SLC_DIR)
