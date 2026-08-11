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

## ADR: Personal deployment is a new script and feature, not an install.sh flag

**ID:** personal-install-separate-script
**Plan:** add-arm64-support
**Status:** Accepted

### Context

Exasol Personal exposes no BucketFS HTTP endpoint, so the standard `scripts/install.sh` upload path (`exapump bucketfs cp` to port 2581) cannot reach it. Personal instead requires SSH transport, filesystem-level BucketFS reconciliation, and `ALTER SYSTEM` (not `ALTER SESSION`) registration that preserves pre-existing `SCRIPT_LANGUAGES` entries.

### Decision

Add `scripts/install-personal.sh` and a `container/personal-install` feature as a separate path, rather than a `--personal` flag on `scripts/install.sh`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Separate script and feature | ✓ Chosen — the transport (SSH + filesystem) and registration scope (`ALTER SYSTEM`, entry preservation) differ fundamentally from the upload+`ALTER SESSION` path |
| `--personal` flag on `install.sh` | ✗ Rejected — would branch one script across two incompatible transports, blurring its responsibility |

### Consequences

Each script keeps a single, crisp responsibility. The shared `#`-fragment/registration-string assembly logic is extracted into a sourced helper (`scripts/lib/script_languages.sh`) both scripts use, so the executable-path invariant has one owner instead of two independent implementations.

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
