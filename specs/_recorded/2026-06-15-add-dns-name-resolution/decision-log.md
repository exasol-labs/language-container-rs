# Decision Log: add-dns-name-resolution

Date: 2026-06-15

## Interview

No clarifying interview was required. The user supplied a fully-specified scratch plan (`PLAN-dns-name-resolution.md`) with every decision pre-made. The key inputs taken as given:

**Q:** SCALAR or SET for `resolv_udf`?
**A:** SCALAR — it does pure `getaddrinfo`, no connect-back, no CONNECTION object, no session, so there is no SIGABRT risk. SET is only mandated for connect-back UDFs.

**Q:** How are the `/conf/` resolver symlinks produced?
**A:** Staging-dir `tar` inside the Docker build (Alpine's own pinned `tar`), not a host `python3` patch. Verified working 2026-06-15 (throwaway build produced `lrwxrwxrwx ./etc/{hosts,resolv.conf}` entries).

**Q:** How does the IT harness obtain the SLC?
**A:** Read `SLC_TARBALL`; delete `export_image_filesystem()` (and the never-committed python-patch path); fail fast with clear guidance if `SLC_TARBALL` is unset.

**Q:** New crate name and prior art?
**A:** New `resolv-udf` crate in `test-udfs/`. There is no existing name-resolution crate — the branch is a clean slate.

**Q:** Versioning?
**A:** Bump workspace `0.8.0 → 0.9.0`; the CI release job auto-tags on merge when the version is new.

## Design Decisions

### [1] Resolver symlinks produced by in-build staging-dir `tar`, superseding any host-`python3` patch

- **Decision:** Produce `/etc/hosts → /conf/hosts` and `/etc/resolv.conf → /conf/resolv.conf` inside the Docker build, in a `packager` stage that copies the runtime root into a staging dir, runs `ln -sf` there, and `tar`s it with Alpine's pinned `tar`. An `artifact` stage (`FROM scratch`) exposes the gzip for `docker build --target artifact --output type=local`. `python3` leaves the SLC packaging path entirely.
- **Alternatives:** (a) Live symlink at `/etc/hosts`/`/etc/resolv.conf` in the image — rejected: `COPY` dereferences the dangling symlink into a 0-byte file and `RUN ln -sf` hits Docker's build-time bind-mount of those two paths. (b) Host-side `python3` tarball patch invoked from install.sh, the IT harness, and ci.yml (the prior, never-committed approach) — rejected: undeclared `python3` dependency plus triplicated, drift-prone logic.
- **Rationale:** The staging dir is not bind-mounted (so `ln` works) and `tar` records symlinks as-is (no `COPY` dereference). Verified 2026-06-15 to produce byte-for-byte the same symlink entries the python script produced. One self-contained artifact replaces three host-side patch sites. This supersedes the prior host-patch consequence captured in the project memory note `slc-tarball-symlink-patch.md`, and aligns with ADR-036 (specs/decision-log.md): the spec scenarios assert observable tarball behavior, while the Dockerfile stage names, `ln -sf` lines, and `docker build` flags live only here and in `plan.md`.
- **Promotes to ADR:** yes

### [2] SLC distribution tarball is the build artifact; all consumers read it, none patch

- **Decision:** Make `slc-rs.tar.gz` (from the `artifact` stage) the single artifact. The IT harness reads it via `SLC_TARBALL`; `install.sh`, `ci-it-local.sh`, and `ci.yml` produce it with `docker build --target artifact --output` and upload/consume it. Drop `docker save`/`docker load`/`docker export` per-consumer steps.
- **Alternatives:** Keep `docker save` (CI image artifact) + per-consumer `docker export | gzip` — rejected: every consumer reran the export step, which drifts and duplicates the flatten-and-gzip logic.
- **Rationale:** A single, already-patched, self-contained artifact every consumer reads identically eliminates drift and the export round-trip.
- **Promotes to ADR:** yes

### [3] IT harness reads `SLC_TARBALL` and fails fast if unset

- **Decision:** Rewrite `load_slc()` to read the `SLC_TARBALL` file and upload it; error with build guidance if unset. Delete `export_image_filesystem()` and the `SLC_IMAGE` constant.
- **Alternatives:** Keep the `docker create`/`docker export` fallback — rejected: a silent fallback to docker/python hides a setup mistake and reintroduces the dependency the plan removes.
- **Rationale:** Local dev builds the tarball once (via `ci-it-local.sh`); a missing `SLC_TARBALL` is a setup error that should surface loudly.
- **Promotes to ADR:** no

### [4] `resolv_udf` is SCALAR and hard-errors on resolution failure

- **Decision:** SCALAR `resolv_udf(host VARCHAR) RETURNS VARCHAR`; resolves `format!("{host}:0")` via `ToSocketAddrs`, returns the first IP, returns a hard `UdfError` on failure.
- **Alternatives:** (a) SET/EMITS — unnecessary; no connect-back, so the SET-only rule for connect-back UDFs does not apply. (b) Return NULL/empty on failure — rejected: silently masking a DNS misconfig defeats the gate's purpose.
- **Rationale:** Pure `getaddrinfo` carries no SIGABRT risk; a hard error makes the gate a real DNS check.
- **Promotes to ADR:** no

### [5] Out-of-scope boundaries held

- **Decision:** The base64 password-decode `python3` one-liners in `ci-it-local.sh`/`ci.yml` are left untouched; no musl resolver work is attempted (the binary is glibc-linked).
- **Alternatives:** Sweep all `python3` usage in one change — rejected: the password-decode one-liner is a separate concern from the symlink patch and out of scope here.
- **Rationale:** Keeps the change focused on the DNS/packaging path; the remaining `python3 -c` base64 decode can be swapped for `base64 -d` later independently.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
