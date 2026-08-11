# Decisions: add-arm64-support

## ADR: License targets cover the glibc triple for both architectures

**ID:** license-targets-cover-glibc-triples
**Plan:** add-arm64-support
**Status:** Accepted

### Context

`about.toml`'s `targets` array selects which triples cargo-about evaluates when generating `THIRD-PARTY-LICENSES.md`. The prior list pinned only `aarch64-unknown-linux-musl`-shaped entries, but the shipped `exaudfclient` is built glibc (`rust:1.94-bookworm`, no `--target`), so the musl-only pin already under-reported `gnu`-gated dependencies before arm64 support existed. cargo-about evaluates `targets` as a union: a dependency reached only through a `cfg(...)` gate is attributed if it matches any listed target, so omitting a shipped architecture silently drops that architecture's gated dependencies — an attribution defect — while listing extra targets only over-attributes.

### Decision

Set `targets` to `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`.

### Options Considered

| Option | Verdict |
|--------|---------|
| All four triples (both libc, both arches) | ✓ Chosen — fixes the latent gnu gap and adds aarch64 in one move; union makes over-attribution safe |
| Add only `aarch64-unknown-linux-musl` (the issue's literal ask) | ✗ Rejected — leaves the pre-existing gnu-gated under-attribution unfixed |

### Consequences

`THIRD-PARTY-LICENSES.md` attributes every dependency gated on any of the four triples for both shipped architectures. Over-attribution is compliance-safe; the fix is packaging-only and changes no runtime behavior.

## ADR: Remove the vestigial targets/*.json build-std files

**ID:** remove-vestigial-target-spec-json
**Plan:** add-arm64-support
**Status:** Accepted

### Context

`targets/*-dylib.json` files exist in the repo, referenced only by a `COPY targets/ ./targets/` line in `Dockerfile.alpine`. Neither the CLI nor the Dockerfile builds with `-Zbuild-std` against these files — both build with rustup triple strings. No in-repo rustc invocation consumes them.

### Decision

Delete `targets/*-dylib.json` and the `COPY targets/` line in `Dockerfile.alpine`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Delete the dead files and the Dockerfile reference | ✓ Chosen — no consumer exists; keeping dead config invites the false assumption that it is load-bearing |
| Keep as a documented manual `-Zbuild-std` path | ✗ Rejected — nothing in the repo exercises this path; documentation without a consumer still rots |

### Consequences

The Dockerfile and repo carry no unreferenced target-spec JSON. Removal rests solely on the absence of a consumer; the JSON's `data-layout` is not stale relative to `rustc 1.94.1`, so no rustc-output re-verification is needed on future toolchain bumps.

## ADR: x86_64 keeps the unsuffixed release tarball name; aarch64 is suffixed

**ID:** aarch64-release-tarball-suffix
**Plan:** add-arm64-support
**Status:** Accepted

### Context

The release pipeline publishes one tarball per architecture. `docs/installation.md` and the manual-install `curl` URL already embed the unsuffixed x86_64 name (`lc-rust-<version>.tar.gz`). The release workflow's `files: lc-rust-*.tar.gz` glob matches any suffix.

### Decision

Keep `lc-rust-<version>.tar.gz` for x86_64; publish `lc-rust-<version>-aarch64.tar.gz` for arm64.

### Options Considered

| Option | Verdict |
|--------|---------|
| Suffix only aarch64, keep x86_64 unsuffixed | ✓ Chosen — non-breaking; existing documented links keep working |
| Suffix both (`-x86_64` / `-aarch64`) | ✗ Rejected — breaks the documented unsuffixed URL for no benefit |

### Consequences

Existing install docs and links need no change. The naming is asymmetric across architectures, a small and permanent cost for a non-breaking release.

## ADR: Integration tests stay x86_64-only; arm64 end-to-end is manual

**ID:** arm64-ci-unit-only-no-integration-leg
**Plan:** add-arm64-support
**Status:** Accepted

### Context

`exasol/docker-db` publishes amd64-only images. Running the privileged, multi-GB DB container under QEMU emulation to exercise an arm64 integration leg is impractical.

### Decision

Add an arm64 CI leg that builds the workspace and runs unit tests only. The integration suite (`cargo test -p it --features integration`) remains x86_64-only. arm64 end-to-end verification is manual, run against Exasol Personal, until an arm64 `docker-db` image exists.

### Options Considered

| Option | Verdict |
|--------|---------|
| arm64 CI leg: build + unit tests only | ✓ Chosen — matches what CI can actually exercise without emulating a privileged DB |
| Full arm64 integration leg via QEMU | ✗ Rejected — `exasol/docker-db` is amd64-only; QEMU-emulating the privileged DB is a non-starter |

### Consequences

CI catches arm64 build and unit-test regressions automatically; an operator must manually re-verify end-to-end UDF execution on Personal after any change touching the platform-specific paths, until an arm64 DB image ships.

## ADR: Personal deployment routes `--deployment` on the descriptor's `.backend` field

**ID:** personal-install-deployment-flag
**Plan:** add-personal-cloud-install
**Status:** Accepted

### Context

Exasol Personal exposes no BucketFS HTTP endpoint for a local (Apple Silicon VM) deployment, so the standard `scripts/install.sh` upload path (`exapump bucketfs cp` to port 2581) cannot reach it there. Local Personal instead requires SSH transport, filesystem-level BucketFS reconciliation, and `ALTER SYSTEM` (not `ALTER SESSION`) registration that preserves pre-existing `SCRIPT_LANGUAGES` entries. A Personal deployment can also run on a cloud backend (`aws`/`azure`/`exoscale`/`stackit`) that reaches the DB over the network and exposes the ordinary BucketFS HTTP endpoint; for a cloud backend, the SSH/filesystem path is wrong and the ordinary HTTP path is correct. The Personal launcher itself already discriminates these cases via `IsLocalBackend()`, keyed on the deployment descriptor's `.backend` field.

### Decision

Handle Personal as a `--deployment <name>` mode of `scripts/install.sh` that reads `deployment.json` `.backend` and branches: `local` selects the SSH/filesystem special path; any other value selects the standard BucketFS HTTP path with connection details harvested from the deployment directory (`deployment.json` `.connection.*` for host/port/user, `secrets.json` `.dbPassword` for the DB password; CLI flags override); a missing or empty `.backend` fails with a clear error. Personal provisions no BucketFS password on either backend, so the operator supplies `--bfs-password`. The `container/personal-install` feature spec describes both paths.

### Options Considered

| Option | Verdict |
|--------|---------|
| `--deployment` mode on `install.sh`, discriminated on `.backend` | ✓ Chosen — build (license bundle + `docker build`), tarball reporting, and the `#`-fragment registration-string assembly are identical across backends; a single script keeps them defined once. `.backend` is what the launcher itself keys off, so the descriptor is authoritative and the operator adds no flag. The cloud case needs no new transport — it is the same HTTP `else` branch as a non-`--deployment` install, only pre-filled from the descriptor. |
| Separate `install-personal.sh` script | ✗ Rejected — duplicated the entire build/report scaffold and forced the shared registration-string helper (`scripts/lib/script_languages.sh`) to exist solely to keep two scripts from drifting; the divergence is one transport step, not a whole second tool. |
| A `--cloud`/`--local` CLI flag, or a hardcoded allowlist of cloud backend names | ✗ Rejected — a flag adds operator burden and can contradict the descriptor; an allowlist is brittle as new cloud backends appear. `.backend != "local"` is the durable test, and erroring on empty/absent `.backend` avoids silently guessing a transport for a malformed descriptor. |

### Consequences

`install.sh` owns all three transport shapes (non-Personal HTTP, local Personal SSH/filesystem, cloud Personal HTTP) behind one `.backend`-keyed branch. The non-`--deployment` HTTP path is unchanged. `--deployment` on `local` forces the VM SQL host/port, uses `ALTER SYSTEM`, and preserves pre-existing entries, exactly as before. `--deployment` on a cloud backend resolves connection details from the deployment directory and falls through to the same HTTP `else` branch as the non-Personal path — no dedicated cloud transport exists to keep in sync. The `#`-fragment/registration-string assembly remains in the sourced helper `scripts/lib/script_languages.sh` (single owner of the executable-path invariant, and the seam the Personal-path unit tests source). A `--deployment` descriptor with no `.backend` field now fails with a clear error instead of silently taking the local path — the one behavior change from the original single-mode design.

## ADR: The cloud Personal path reuses the existing HTTP branch unchanged

**ID:** personal-cloud-reuses-http-transport
**Plan:** add-personal-cloud-install
**Status:** Accepted

### Context

A cloud Personal deployment already exposes the ordinary BucketFS HTTP endpoint; the deployment directory (`deployment.json` `.connection.*`, `secrets.json` `.dbPassword`) supplies only the connection details an operator would otherwise type by hand. The transport itself — `exapump bucketfs cp` upload plus `ALTER <scope> SET SCRIPT_LANGUAGES` — is already the default, already-tested path for every non-Personal install.

### Decision

For a cloud backend, `resolve_cloud_connection` fills the normal-path connection variables (`HOST`/`PORT`/`USER`/`PASSWORD`) from `deployment.json` and `secrets.json`, honoring `--host`/`--port`/`--user`/`--password` overrides, then control falls through to the existing HTTP `else` branch with no code change. `--bfs-password` is required, because Personal provisions no BucketFS password on any backend. `--scope` keeps its default `SESSION`; no `SYSTEM` scope is forced and no entry-preservation read-merge-write applies.

### Options Considered

| Option | Verdict |
|--------|---------|
| Reuse the existing HTTP `else` branch verbatim after resolving credentials | ✓ Chosen — cloud is the normal path; the deployment directory only supplies credentials, so a second implementation would be duplicate risk with no benefit. One HTTP branch stays the single owner of the upload-and-register logic. |
| A dedicated cloud transport function paralleling the local one | ✗ Rejected — the wire transport is already correct and already verified operationally; a parallel implementation could drift from the branch it duplicates. |

### Consequences

Cloud Personal installs get every future improvement to the shared HTTP branch for free, with zero cloud-specific transport code to maintain. The cloud path differs from the non-Personal HTTP path only in where connection details originate (descriptor vs. command line) and in the mandatory `--bfs-password` check.

## ADR: Preserve field-verified arm64/Personal platform knowledge as normative spec clauses

**ID:** preserve-arm64-platform-knowledge
**Plan:** add-arm64-support
**Status:** Accepted

### Context

Several field-debugging sessions established platform facts whose violation produces an opaque `22002 VM crashed` failure with no direct link back to the root cause: the staged glibc loader must land at the binary's exact `PT_INTERP` path (Alpine's `/lib` is a real directory, not a symlink to `/usr/lib`); the shipped `exaudfclient` is glibc, not musl; the `SCRIPT_LANGUAGES` `#` fragment must name the `exaudfclient` executable with no leading slash; Personal's SSH port changes on every `exasol start` and must be read fresh; and integration tests stay x86_64 because `exasol/docker-db` is amd64-only.

### Decision

Encode each fact as a normative (MUST) clause in the relevant spec scenario (`container/slim-image`, `container/personal-install`) rather than leaving it recoverable only from issue or PR history.

### Options Considered

| Option | Verdict |
|--------|---------|
| Encode as normative spec clauses | ✓ Chosen — cheapest insurance against silent regression of facts that each took a field session to establish |
| Leave in issue/PR history only | ✗ Rejected — not surfaced to an implementer working from the spec library, and easy to silently regress |

### Consequences

An implementer changing the Dockerfile or install scripts sees the invariant directly in the spec they are editing, with the failure mode named, instead of rediscovering it through a live `22002` crash.
