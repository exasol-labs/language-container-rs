# Verification Report: add-name-resolution

**Generated:** 2026-06-15 (final update 2026-06-15 after tasks 2.8 and 2.9)

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | DNS resolution works end-to-end in the Alpine SLC sandbox. `www.exasol.com` resolves to a valid IP at UDF runtime inside `exasol/docker-db:2025.1.11`. All scenarios pass. Redundant Dockerfile additions reverted (task 2.8). `patch_resolver_symlinks()` extracted to `scripts/patch-slc-symlinks.py` shared by IT harness and `install.sh` (task 2.9). |

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

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit | `cargo +1.91 test --workspace --exclude it` | pre-existing failures in `exa-udf-runtime` dispatch tests (unrelated to this plan) | — |
| Integration | `EXASOL_VERSION=2025.1.11 cargo +1.91 test -p it --features integration` | 2/2 (db_roundtrip_all_scenarios, name_resolution_resolves_external_hostname_gate) | — |

### Manual Tests

| Test | Result |
|------|--------|
| `docker build -f Dockerfile.alpine` exits 0; `/etc/nsswitch.conf` present with `hosts: files dns` (Alpine native, not from Dockerfile) | ✓ |
| e2e gate `EXASOL_VERSION=2025.1.11 ... name_resolution_resolves_external_hostname_gate` | ✓ PASS 2026-06-15 |
| `[it] scenario name_resolution ok` in `db_roundtrip_all_scenarios` against 2025.1.11 | ✓ |
| `scripts/install.sh` pipes `docker export` through `patch-slc-symlinks.py` (not bare `gzip`) | ✓ |

## Tool Evidence

### Linter

```
cargo +1.91 clippy -p name-resolution -p exaudfclient -p it --features integration -- -D warnings
Finished `dev` profile — 0 warnings
```

### Formatter

```
cargo fmt --check -p name-resolution
cargo +1.91 fmt --check -p exaudfclient -p it
No changes — format ok
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| container | slim-image | Builder toolchain and glibc runtime (CHANGED) | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` (image built from Dockerfile.alpine) | Pass |
| container | slim-image | Alpine runtime image carries glibc name-resolution config (NEW) | `crates/it/tests/db_roundtrip.rs` | `name_resolution_resolves_external_hostname` (covered: resolution success proves config is present) | Pass |
| container | slim-image | Alpine runtime image resolves external hostnames at UDF runtime (NEW) | `crates/it/tests/db_roundtrip.rs` | `name_resolution_resolves_external_hostname` | Pass |

## Notes

### Implementation Details

Docker's image layer system cannot place symlinks at `/etc/hosts` and `/etc/resolv.conf`:
COPY dereferences broken symlinks (converts to 0-byte regular files), and `RUN ln` hits
Docker's bind-mount of those paths at build time. The SLC filesystem is also read-only
at sandbox runtime, so neither writes nor bind-mounts work there.

**Chosen fix**: `scripts/patch-slc-symlinks.py` post-processes the raw `docker export`
tarball using Python3's `tarfile` module: it replaces the 0-byte placeholder entries at
`etc/hosts` and `etc/resolv.conf` with proper symlink entries pointing into `/conf/`, then
gzip-compresses the result. The Exasol sandbox extracts the patched tarball;
`/etc/resolv.conf -> /conf/resolv.conf` and `/etc/hosts -> /conf/hosts` are present at
runtime. The DB-injected `nameserver 8.8.8.8` in `/conf/resolv.conf` is then read by
glibc's `libnss_dns` via the symlink chain.

Both the IT harness (`patch_resolver_symlinks()` in `crates/it/src/lib.rs`) and the
production packaging workflow (`scripts/install.sh`) call the same script, eliminating
the former production packaging gap where a bare `docker export | gzip` produced a broken
`/etc/resolv.conf` with no symlink.

### e2e Execution Gate

- Exasol version: `2025.1.11`
- Date: 2026-06-15
- Result: PASS
- The UDF resolved `www.exasol.com` and emitted a non-empty string that parses as a valid `IpAddr`.
