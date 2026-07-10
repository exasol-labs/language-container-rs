# Decisions: add-dns-name-resolution

## ADR: Resolver symlinks produced by in-build staging-dir tar, superseding host-python3 patch

**ID:** resolver-symlinks-staging-dir-tar
**Plan:** `add-dns-name-resolution`
**Status:** Accepted

### Context

Exasol UDFs run in a sandbox that bind-mounts the database's resolver config at `/conf/`. For DNS to work, the SLC root filesystem must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`. Two failure modes block creating these symlinks in the image directly: `COPY` dereferences a dangling symlink into a 0-byte file, and `RUN ln -sf /conf/... /etc/...` hits Docker's build-time bind-mount of those two paths. A prior, never-committed approach worked around this with a host-side `python3` tarball patch invoked from three places (install script, IT harness, CI), introducing an undeclared `python3` dependency and triplicated, drift-prone logic.

### Decision

Produce `/etc/hosts → /conf/hosts` and `/etc/resolv.conf → /conf/resolv.conf` inside the Docker build, in a `packager` stage that copies the runtime root into a staging directory, runs `ln -sf` there (the staging dir is NOT bind-mounted, so `ln` succeeds), and `tar`s it with Alpine's own pinned `tar` (which records symlinks as-is, with no `COPY` dereference). An `artifact` stage (`FROM scratch`) exposes the resulting `lc-rs.tar.gz` for `docker build --output`. `python3` leaves the SLC packaging path entirely. Verified 2026-06-15 to produce byte-for-byte the same symlink entries the python script produced.

### Options Considered

| Option | Verdict |
|--------|---------|
| Staging-dir `tar` inside the Docker build (`packager` stage) | ✓ Chosen — staging dir is not bind-mounted (so `ln` works) and `tar` records symlinks as-is; verified to produce identical entries; one self-contained location for all packaging logic |
| Live symlink at `/etc/hosts`/`/etc/resolv.conf` in the image | ✗ Rejected — `COPY` dereferences the dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths |
| Host-side `python3` tarball patch (prior, never-committed approach) | ✗ Rejected — undeclared `python3` dependency plus triplicated, drift-prone logic invoked from install.sh, IT harness, and CI |

### Consequences

The SLC tarball has proper symlink entries (`lrwxrwxrwx ./etc/resolv.conf -> /conf/resolv.conf`, `lrwxrwxrwx ./etc/hosts -> /conf/hosts`) baked in by the Docker build with no host-side post-processing. DNS resolves inside the UDF sandbox. The project memory note `slc-tarball-symlink-patch.md` is superseded — the host-patch approach is no longer used. Spec scenario `container/slim-image / SLC tarball ships the /conf resolver symlinks` asserts the observable tarball behavior.

## ADR: SLC distribution tarball is the build artifact; all consumers read it, none patch

**ID:** slc-tarball-is-the-build-artifact
**Plan:** `add-dns-name-resolution`
**Status:** Accepted

### Context

Previously each consumer of the SLC (install.sh, IT harness, CI) independently ran `docker save`/`docker export | gzip` to produce a flattened tarball, and the now-superseded approach additionally required a host-side python3 patch step. Every consumer reran the export step, duplicating and drifting the flatten-and-gzip logic.

### Decision

Make `lc-rs.tar.gz` (produced by the `artifact` stage with `docker build --target artifact --output type=local,...`) the single build artifact. The IT harness reads it via `SLC_TARBALL`; `install.sh`, `ci-it-local.sh`, and `ci.yml` produce it with `docker build --target artifact --output` and upload/consume it directly. `docker save`/`docker load`/`docker export` per-consumer steps are dropped.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single `lc-rs.tar.gz` artifact from `artifact` stage; all consumers read it | ✓ Chosen — one source of truth; eliminates drift; no per-consumer export round-trip; symlinks already baked in (ADR-037) |
| Keep `docker save` (CI image artifact) + per-consumer `docker export \| gzip` | ✗ Rejected — every consumer reran the export step, which drifts and duplicates the flatten-and-gzip logic; does not solve the symlink problem |

### Consequences

A single, self-contained `lc-rs.tar.gz` artifact is produced once and read by every consumer without modification. The IT harness reads the path from `SLC_TARBALL` and fails fast with clear guidance if it is unset — a missing tarball is a setup error, not a condition to recover from silently. The `docker save`/`docker load`/`docker export` steps are removed from all consumers.
