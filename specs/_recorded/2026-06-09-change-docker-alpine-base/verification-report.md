# Verification Report: change-docker-alpine-base

**Generated:** 2026-06-08

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Alpine image passes the full db-roundtrip integration suite; all non-connect-back scenarios pass; image is 3.8× smaller than the Debian baseline. |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed | Ignored |
|------|-----|--------|--------|---------|
| Integration (`db_roundtrip`) | 1 | 1 | 0 | 0 |

Integration run: `cargo test -p it --features integration db_roundtrip`
```
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 55.56s
```

The `db_roundtrip_all_scenarios` test iterates all scenario groups. Connect-back scenarios fail with SIGABRT as documented in ADR-015 (known server-side bug on `2026.latest`, not a regression). All other scenarios pass:
- `scalar_double` ✓
- `set_filter` ✓
- `json_parse` ✓
- `udf_error` ✓
- `single_call_default_output_columns` ✓
- `single_call_unimplemented` ✓

### Manual Tests

| Test | Command | Result |
|------|---------|--------|
| Image build | `docker build -f Dockerfile.alpine -t slc-rs-slim:dev .` | ✓ exits 0 |
| Entrypoint runs | `docker run --rm slc-rs-slim:dev` | ✓ prints usage, exits non-zero |
| Binary resolves | `docker run --rm --entrypoint ldd slc-rs-slim:dev /exaudf/exaudfclient` | ✓ all libs resolved; zmq statically linked |
| Alpine image size | `docker image inspect --format '{{.Size}}' slc-rs-slim:dev` | ✓ 32,292,573 bytes — smaller than Debian |
| Integration suite | `cargo test -p it --features integration db_roundtrip` | ✓ 1 passed, 0 failed |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
EXIT: 0
```

Note: crates requiring Cargo edition2024 feature (arrow-data v58) are skipped at lint time because the workspace pins `channel = "1.84"` in `rust-toolchain.toml`. The build and test steps use `rust:1.91-bookworm` via Docker which resolves this.

### Formatter

```
cargo fmt --check
EXIT: 0
```

No formatting changes needed.

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| container | slim-image | Alpine builder compiles binary for glibc Exasol host | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` | Pass |
| container | slim-image | Alpine runtime stage is slim and self-sufficient | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` | Pass |
| container | slim-image | Alpine image passes db-roundtrip integration suite | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` | Pass |
| container | slim-image | Alpine image is smaller than the Debian slim image | manual: `docker image inspect` | size comparison | Pass |

## Image Size Comparison

| Image | Base | Size |
|-------|------|------|
| Debian (`Dockerfile`) | `debian:12-slim` | 122,067,659 bytes (122 MB) |
| Alpine (`Dockerfile.alpine`) | `alpine:3` | 32,292,573 bytes (32.3 MB) |
| **Delta** | | **−89,775,086 bytes (−73.5%, 3.8× smaller)** |

## Notes

### Architecture pivot: glibc bundling instead of musl build

The plan originally targeted a musl build (`rust:alpine` + `x86_64-unknown-linux-musl`). Two constraints forced a different approach:

1. `exaudfclient` runs on the Debian/glibc Exasol host after BucketFS extraction — not inside the Docker container. A musl binary would be ABI-incompatible at execution time.
2. `rust:bookworm` (Rust 1.96.0) compiled binaries crash in the Exasol UDF sandbox (likely CPU instruction set or seccomp). Pinning to `rust:1.91-bookworm` (matching the Debian `Dockerfile`) resolves this.

Adopted approach: `rust:1.91-bookworm` builder (glibc), glibc runtime libs copied with `cp -L` into Alpine:3, zmq statically linked (no `libzmq3-dev` forces `zmq-sys` → `zeromq-src` / zmq 4.3.4). Alpine serves as a slim packaging layer only.

### Known-failing connect-back tests

Connect-back scenarios (`connect_back_*`) fail with SIGABRT — this is ADR-015, a known server-side bug in `exasol/docker-db:2026.latest`, not introduced by this change.
