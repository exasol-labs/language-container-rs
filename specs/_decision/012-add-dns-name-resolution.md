# Decisions: add-dns-name-resolution

## ADR: Resolver symlinks produced by in-build staging-dir tar

**ID:** resolver-symlinks-staging-dir-tar
**Plan:** `add-dns-name-resolution`
**Status:** Accepted

### Context

The UDF sandbox bind-mounts the database resolver config at `/conf/`, so the SLC must ship `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`. `COPY` dereferences a dangling symlink into a 0-byte file, and `RUN ln -sf` hits Docker's build-time bind-mount of those two paths.

### Decision

The Docker build creates `/etc/hosts → /conf/hosts` and `/etc/resolv.conf → /conf/resolv.conf` in the `staging` stage. The stage copies the runtime root into a staging directory, runs `ln -sf` there, and archives it with that stage's `tar`, which records symlinks as-is. An `artifact` stage (`FROM scratch`) exposes `lc-rs.tar.gz` for `docker build --output`. No host-side tool touches the tarball.

### Options Considered

| Option | Verdict |
|--------|---------|
| Staging-dir `tar` in the Docker build | ✓ Chosen |
| Live symlink in the image | ✗ `COPY` dereferences it; `RUN ln -sf` hits the bind-mount |
| Host-side `python3` tarball patch | ✗ Undeclared `python3` dependency; logic duplicated across install.sh, IT harness and CI |

### Consequences

The tarball carries the symlink entries without post-processing. Spec scenario `container/slim-image / SLC tarball ships the /conf resolver symlinks` asserts this.

## ADR: SLC distribution tarball is the build artifact; all consumers read it, none patch

**ID:** slc-tarball-is-the-build-artifact
**Plan:** `add-dns-name-resolution`
**Status:** Accepted

### Context

Each SLC consumer (install.sh, IT harness, CI) would otherwise run its own export and flatten step, duplicating logic that drifts.

### Decision

`lc-rs.tar.gz`, produced by `docker build --target artifact --output type=local,...`, is the single build artifact. The IT harness reads it via `SLC_TARBALL` and fails fast if the variable is unset. `install.sh`, `ci-it-local.sh` and `ci.yml` produce and consume it directly. No consumer runs `docker save`, `docker load` or `docker export`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single `lc-rs.tar.gz` from the `artifact` stage | ✓ Chosen |
| `docker save` plus per-consumer `docker export \| gzip` | ✗ Each consumer repeats the export; logic drifts |

### Consequences

A missing tarball is a setup error, not a condition to recover from.
