# Decisions: add-arm64-support

## ADR: License targets cover the glibc triple for both architectures

**ID:** license-targets-cover-glibc-triples
**Plan:** add-arm64-support
**Status:** Accepted

### Context

cargo-about evaluates the `targets` array in `about.toml` as a union when generating `THIRD-PARTY-LICENSES.md`. The shipped `exaudfclient` and UDF `.so` files are glibc builds, so a missing shipped triple drops its gated dependencies and an unshipped triple adds dependencies that never ship.

### Decision

`targets` lists exactly `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. `dist/tests/about_toml_test.sh` fails the build if either `-unknown-linux-musl` triple appears.

### Options Considered

| Option | Verdict |
|--------|---------|
| Both glibc triples only | ✓ Chosen |
| All four triples (both libc, both architectures) | ✗ Attributes musl-only dependencies that never ship |
| `x86_64-unknown-linux-musl` only | ✗ Musl is not shipped and aarch64 is missing |

### Consequences

`THIRD-PARTY-LICENSES.md` attributes dependencies gated on either glibc triple and no musl-only entries.

## ADR: Personal deployment routes `--deployment` on the descriptor's `.backend` field

**ID:** personal-install-deployment-flag
**Plan:** add-personal-cloud-install
**Status:** Accepted

### Context

A local Exasol Personal deployment (Apple Silicon VM) exposes no BucketFS HTTP endpoint. It needs SSH transport, filesystem-level BucketFS reconciliation and `ALTER SYSTEM` registration that preserves existing `SCRIPT_LANGUAGES` entries. A cloud backend (`aws`/`azure`/`exoscale`/`stackit`) exposes the ordinary BucketFS HTTP endpoint. The Personal launcher already discriminates these cases with `IsLocalBackend()` on the descriptor's `.backend` field.

### Decision

`scripts/install.sh --deployment <name>` reads `deployment.json` `.backend`. `local` selects the SSH/filesystem path. Any other value selects the standard BucketFS HTTP path, with host/port/user from `deployment.json` `.connection.*` and the DB password from `secrets.json` `.dbPassword`; CLI flags override. A missing or empty `.backend` fails with a clear error. Personal provisions no BucketFS password, so the operator supplies `--bfs-password`. The `container/personal-install` feature spec describes both paths.

### Options Considered

| Option | Verdict |
|--------|---------|
| `--deployment` mode on `install.sh`, keyed on `.backend` | ✓ Chosen: build, tarball reporting and registration-string assembly stay defined once |
| Separate `install-personal.sh` | ✗ Duplicates the build/report scaffold; the difference is one transport step |
| `--cloud`/`--local` flag or hardcoded cloud-backend allowlist | ✗ A flag can contradict the descriptor; an allowlist is brittle |

### Consequences

`install.sh` owns three transport shapes (non-Personal HTTP, local Personal SSH/filesystem, cloud Personal HTTP) behind one `.backend` branch. Cloud Personal reuses the non-Personal HTTP branch with variables pre-filled from the descriptor. The registration-string assembly lives in the sourced helper `scripts/lib/script_languages.sh`, which the Personal-path unit tests also source.
