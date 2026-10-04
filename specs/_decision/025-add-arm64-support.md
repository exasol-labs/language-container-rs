# Decisions: add-arm64-support

## ADR: License targets cover the glibc triple for both architectures

**ID:** license-targets-cover-glibc-triples
**Plan:** add-arm64-support
**Status:** Accepted

### Context

`about.toml`'s `targets` array selects which triples cargo-about evaluates when generating `THIRD-PARTY-LICENSES.md`. The prior list pinned only `x86_64-unknown-linux-musl`, but the shipped `exaudfclient` is built glibc (`rust:1.94-bookworm`, no `--target`) and the UDF `.so`s it loads are glibc-dynamic cdylibs — nothing musl is distributed. The musl-only pin therefore both under-reported the shipped `gnu`-gated dependencies and over-reported musl-only ones. cargo-about evaluates `targets` as a union: a dependency reached only through a `cfg(...)` gate is attributed if it matches any listed target, so omitting a shipped architecture silently drops that architecture's gated dependencies (an attribution defect), while listing an unshipped triple over-attributes dependencies that never ship.

### Decision

Set `targets` to the two glibc triples for the shipped architectures — `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` — and nothing else. `dist/tests/about_toml_test.sh` enforces this: the build fails if either `-unknown-linux-musl` triple appears.

### Options Considered

| Option | Verdict |
|--------|---------|
| Both glibc (gnu) triples only, no musl | ✓ Chosen — attributes exactly the shipped architectures; a committed test forbids musl so the manifest can't over-attribute unshipped deps |
| All four triples (both libc, both arches) | ✗ Rejected — over-attributes musl-only dependencies that never ship, since nothing musl is built |
| Keep only `x86_64-unknown-linux-musl` (main's prior pin) | ✗ Rejected — musl isn't shipped, and it omits aarch64, under-reporting the shipped glibc deps |

### Consequences

`THIRD-PARTY-LICENSES.md` attributes every dependency gated on either shipped glibc triple, for both architectures, and no musl-only entries. The fix is packaging-only and changes no runtime behavior.

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

For a cloud backend, `install.sh` fills the `HOST`/`PORT`/`USER`/`PASSWORD` variables from `deployment.json` and `secrets.json` (CLI flags override) and reuses the existing HTTP branch unchanged with no cloud-specific transport.
