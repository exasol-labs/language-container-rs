# Plan: add-dns-name-resolution

## Summary

Make the Alpine SLC resolve external DNS hostnames at UDF runtime by producing the `/etc/hosts → /conf/hosts` and `/etc/resolv.conf → /conf/resolv.conf` symlinks entirely inside the Docker build (staging-dir `tar`, no host `python3`), and prove DNS end-to-end with a new SCALAR `resolv_udf` UDF callable as `SELECT resolv_udf('www.exasol.com')`.

## Design

### Context

Exasol UDFs run in a sandbox that bind-mounts the database's resolver config at `/conf/`. For DNS to work, the SLC root filesystem must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`. Two failure modes block creating these symlinks in the image directly: `COPY` dereferences a dangling symlink into a 0-byte file, and `RUN ln -sf /conf/... /etc/...` hits Docker's build-time bind-mount of those two paths. A prior, never-committed approach worked around this with a host-side `python3` tarball patch invoked from three places (install script, IT harness, CI), introducing an undeclared `python3` dependency and triplicated, drift-prone logic.

- **Goals** — DNS resolves inside the UDF sandbox; the resolver symlinks are produced exactly once, inside the Docker build; a parameterized `resolv_udf` gives a one-line end-to-end DNS check; all consumers read one self-contained tarball artifact.
- **Non-Goals** — the base64 password-decode `python3` one-liners in CI/scripts (separate concern); any musl resolver work (the binary is glibc-linked); changing the `language_definitions.json` contract.

### Decision

The SLC distribution tarball — not the Docker image — becomes the build artifact, with the resolver symlinks already baked in. A `packager` stage copies the runtime root filesystem into a staging directory (excluding `/etc/hosts`, `/etc/resolv.conf`, and pseudo-filesystems), creates the two `ln -sf` symlinks in the staging tree (which is NOT bind-mounted, so `ln` succeeds), and `tar`s the staging tree with Alpine's own pinned `tar` (which records symlinks as-is, with no `COPY` dereference). An `artifact` stage (`FROM scratch`) exposes the resulting `slc-rs.tar.gz` for `docker build --output`.

Verified 2026-06-15 with a throwaway build: the staging-dir `tar` produces proper symlink entries (`lrwxrwxrwx ./etc/resolv.conf -> /conf/resolv.conf`, `lrwxrwxrwx ./etc/hosts -> /conf/hosts`) — byte-for-byte what the old python script produced.

Every consumer reads this one artifact: the IT harness reads `SLC_TARBALL`; `install.sh`, `ci-it-local.sh`, and `ci.yml` build it with `docker build --target artifact --output type=local,...` and upload/consume it. No consumer patches the tarball; `python3` leaves the SLC packaging path entirely.

#### Architecture

```
Dockerfile.alpine
  builder ─▶ runtime ─▶ packager (staging-dir tar + ln -sf) ─▶ artifact (scratch: slc-rs.tar.gz)
                                                                     │
        docker build --target artifact --output type=local,dest=DIR ▼
                                                            DIR/slc-rs.tar.gz
                                                                     │
            ┌────────────────────────────┬───────────────────────────┴───────────────┐
            ▼                            ▼                                            ▼
  scripts/install.sh          scripts/ci-it-local.sh                       .github/workflows/ci.yml
  (upload to BucketFS)        (set SLC_TARBALL, run it-runner)              (build-slc → slc-tarball artifact;
            │                            │                                  integration sets SLC_TARBALL;
            └────────────────────────────┴──▶ crates/it::load_slc()         release publishes the file)
                                              reads SLC_TARBALL, uploads
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Staging-dir `tar` inside the build | `Dockerfile.alpine` `packager` stage | `/staging/etc` is not bind-mounted (so `ln` works) and `tar` records symlinks as-is (no `COPY` dereference) |
| `FROM scratch` + `--output type=local` | `Dockerfile.alpine` `artifact` stage | extracts a single regular gzip file from the build with no image-export round-trip |
| Single artifact, zero patching | all consumers | one source of truth; eliminates triplicated host-side patch logic and the undeclared `python3` dependency |
| Fail-fast on missing input | `crates/it` `load_slc()` | a missing `SLC_TARBALL` is a setup error; surface it loudly instead of silently shelling out to docker/python |
| SCALAR pure-resolution UDF | `test-udfs/resolv-udf` | `getaddrinfo` only, no connect-back/CONNECTION/session, so SCALAR carries no SIGABRT risk |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|-------------------------|-----------|
| Produce symlinks via staging-dir `tar` inside the Docker build | Host-side `python3` tarball patch (prior, never-committed approach) | Removes an undeclared `python3` dependency and triplicated logic; verified to produce identical symlink entries |
| Make the tarball the build artifact (`artifact` stage + `--output`) | Keep `docker save`/`docker export` in each consumer | One self-contained artifact every consumer reads identically; no per-consumer export step to drift |
| IT harness reads `SLC_TARBALL` and fails fast if unset | Keep the `docker create`/`export` fallback in `load_slc()` | Local dev builds the tarball once via `ci-it-local.sh`; a silent docker/python fallback hides setup mistakes |
| `resolv_udf` is SCALAR | SET/EMITS (mandated for connect-back UDFs) | No connect-back, no CONNECTION object, no session — pure `getaddrinfo`, so SCALAR is safe |
| Hard `UdfError` on resolution failure | Return NULL / empty string | Silently masking a DNS misconfig defeats the purpose of a DNS gate |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/slim-image | CHANGED | `container/slim-image/spec.md` |
| integration/name-resolution | NEW | `integration/name-resolution/spec.md` |
| integration/db-roundtrip | CHANGED | `integration/db-roundtrip/spec.md` |

## Dependencies

- BuildKit (default in modern Docker and CI `buildx`) for `docker build --target artifact --output type=local`.
- Outbound network access from the integration runner so `www.exasol.com` resolves.

## Implementation Tasks

1. **Dockerfile.alpine — packager + artifact stages**
   - [ ] 1.1 Add a `packager` stage `FROM runtime` that copies the root filesystem into a staging dir (excluding `./slc`, `./etc/hosts`, `./etc/resolv.conf`, `./proc`, `./sys`, `./dev`), creates `ln -sf /conf/hosts /slc/etc/hosts` and `ln -sf /conf/resolv.conf /slc/etc/resolv.conf`, and `tar -czf /slc-rs.tar.gz` the staging tree.
   - [ ] 1.2 Add an `artifact` stage `FROM scratch` that `COPY --from=packager /slc-rs.tar.gz /`.

2. **test-udfs/resolv-udf — new SCALAR UDF crate**
   - [ ] 2.1 Create `test-udfs/resolv-udf/Cargo.toml` (cdylib, version `0.1.0`, deps on `exasol-udf-sdk` + `exasol-udf-macros`) and add the crate to the workspace members if the workspace does not auto-glob `test-udfs/*`.
   - [ ] 2.2 Implement `resolv_udf` in `src/lib.rs`: read column 0 as a string host, resolve `format!("{host}:0")` via `std::net::ToSocketAddrs`, emit the first IP as a `Value::String`, return a hard `UdfError` on resolution failure or wrong input type.
   - [ ] 2.3 Add unit tests for the pure path: a resolvable fixture (e.g. `localhost`) yields a parseable IP; an unresolvable host yields `UdfError`; non-string input yields `UdfError`.

3. **crates/it — read SLC_TARBALL, fail fast, delete export fallback**
   - [ ] 3.1 Rewrite `load_slc()` to read the file at `SLC_TARBALL` and upload its bytes; if `SLC_TARBALL` is unset, return an error with clear guidance to build the tarball (`docker build --target artifact --output …` / run `ci-it-local.sh`).
   - [ ] 3.2 Delete `export_image_filesystem()` and the unused `SLC_IMAGE` constant; remove now-dead `docker create`/`export` imports/usages.
   - [ ] 3.3 Add the `resolv-udf` upload + SCALAR `resolv_udf` script registration to the harness, and add the DNS-gate roundtrip test asserting the result parses as `IpAddr`. Add the unresolvable-host negative test.

4. **scripts/install.sh — build artifact instead of export**
   - [ ] 4.1 Replace the `docker create`/`docker export | gzip` block (and any image-build-only step) with `docker build -f Dockerfile.alpine --target artifact --output type=local,dest=<tmpdir> .`, then upload `<tmpdir>/slc-rs.tar.gz` to BucketFS.

5. **scripts/ci-it-local.sh — build artifact, set SLC_TARBALL**
   - [ ] 5.1 Replace the `docker build -t slc-rs-slim:dev` image step with `docker build --target artifact --output type=local,dest=<dir>`, export `SLC_TARBALL=<dir>/slc-rs.tar.gz`, and pass it through to the `it-runner` invocation.

6. **.github/workflows/ci.yml — artifact-based SLC flow**
   - [ ] 6.1 `build-slc` job: replace the `docker save` image artifact with `docker build --target artifact --output type=local,...` producing `slc-rs.tar.gz`, uploaded as a `slc-tarball` artifact (drop `docker save`/`slc-image`).
   - [ ] 6.2 `integration` job: download the `slc-tarball` artifact and set `SLC_TARBALL` for the `it-runner` run; drop the `docker load -i slc-image.tar` step.
   - [ ] 6.3 `release` job: download `slc-tarball` and publish it directly (drop the `docker load` + `docker export | gzip` block; keep the version-tag/release wiring).

7. **scripts/patch-slc-symlinks.py — delete**
   - [ ] 7.1 Confirm the file does not exist (clean branch); if present, delete it. No code references it.

8. **Workspace version bump**
   - [ ] 8.1 Bump the workspace `version` in `Cargo.toml` from `0.8.0` to `0.9.0`.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1 (Dockerfile), 2 (resolv-udf crate), 8 (version bump) |
| Group B | 3 (IT harness), 4 (install.sh), 5 (ci-it-local.sh), 6 (ci.yml), 7 (delete python) |

Sequential dependencies:
- Group A → Group B (the IT harness DNS-gate test needs the `resolv-udf` crate from 2; consumers need the `artifact` stage from 1).

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/it/src/lib.rs::export_image_filesystem` | Replaced by reading the `SLC_TARBALL` artifact |
| Constant | `crates/it/src/lib.rs::SLC_IMAGE` | No longer exported/loaded by the harness |
| Shell block | `scripts/install.sh` (docker create/export\|gzip) | Replaced by `docker build --target artifact --output` |
| Shell block | `.github/workflows/ci.yml` release (docker load + export\|gzip) | Replaced by publishing the `slc-tarball` artifact |
| File | `scripts/patch-slc-symlinks.py` | Host-side patch superseded by in-build staging-dir `tar` (absent on this branch; delete if present) |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| slim-image / SLC tarball ships the /conf resolver symlinks | Integration | `crates/it/src/lib.rs` (or `crates/it/tests/`) | `slc_tarball_has_conf_resolver_symlinks` |
| name-resolution / resolv_udf resolves an external hostname to a valid IP | Integration | `crates/it/tests/` | `resolv_udf_resolves_external_host` |
| name-resolution / resolv_udf surfaces an error for an unresolvable hostname | Integration | `crates/it/tests/` | `resolv_udf_errors_on_unresolvable_host` |
| db-roundtrip / DNS gate resolves an external hostname end-to-end | Integration | `crates/it/tests/` | `db_roundtrip_dns_gate` |

Supporting unit tests (pure computation, no I/O) in `test-udfs/resolv-udf/src/lib.rs`: `resolves_localhost_to_ip`, `errors_on_unresolvable_host`, `errors_on_non_string_input`.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/slim-image | `docker build -f Dockerfile.alpine --target artifact --output type=local,dest=/tmp/slc .` then `tar tzvf /tmp/slc/slc-rs.tar.gz ./etc/hosts ./etc/resolv.conf` | Two `lrwxrwxrwx` symlink entries pointing to `/conf/hosts` and `/conf/resolv.conf` |
| integration/name-resolution | `scripts/install.sh --host localhost --password exasol --bfs-password <pw>` then `exapump sql "SELECT resolv_udf('www.exasol.com')" -d "exasol://sys:exasol@localhost:8563?validateservercertificate=0"` | A single VARCHAR row containing a valid IP address |
| integration/db-roundtrip | `scripts/ci-it-local.sh` | Integration suite passes including the DNS-gate scenario |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt` | No changes |
| Integration | `scripts/ci-it-local.sh` | DNS-gate + all roundtrip scenarios pass |
