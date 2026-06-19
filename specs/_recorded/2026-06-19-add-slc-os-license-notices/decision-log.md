# Decision Log: add-slc-os-license-notices

Date: 2026-06-19

## Interview

**Q:** How should the OS-layer package manifest be produced and kept honest?
**A:** Commit the manifest, no CI gate. Commit `THIRD-PARTY-OS-LICENSES.md` + a `licenses/` directory of canonical SPDX texts + a written source offer; `Dockerfile.alpine` COPYs them into `/exaudf` alongside the existing `LICENSE` and `THIRD-PARTY-LICENSES.md`. The apk package set changes rarely (only on an `apk add` edit or an `alpine:3` bump), so drift is caught in review — no CI staleness gate (do NOT mirror the cargo-about gate for this).

**Q:** Release strategy given 0.13.0 was bumped in main but is being released separately right now?
**A:** Patch bump 0.13.0 → 0.13.1. v0.13.0 (execute_batch + timestamp fixes) is being tagged/released independently; this license work ships as the next patch, 0.13.1. No API change.

## Design Decisions

### [1] Commit a static OS-layer notice bundle with no CI staleness gate

- **Decision:** Ship `THIRD-PARTY-OS-LICENSES.md` + a committed `licenses/` directory of canonical SPDX texts + an inline written source offer. No generator, no CI gate.
- **Alternatives:** Mirror the existing `cargo about` staleness gate for the OS layer; auto-generate the manifest from `/lib/apk/db/installed` at build time.
- **Rationale:** The bundled OS/runtime component set changes only on an explicit `apk add` edit or an `alpine:3`/builder base-image bump — both review-visible. A generator + gate is maintenance cost with little payoff for a slowly-changing set. (Interview decision.)
- **Promotes to ADR:** yes

### [2] Written 3-year source offer for GPL/LGPL components rather than bundling source

- **Decision:** Satisfy GPLv2 §3 / LGPL-2.1 source obligations with an inline written offer (valid 3 years) pointing at upstream source — Alpine aports for the exact apk versions, Debian source packages / snapshot.debian.org for the glibc & gcc-12 runtime. Permissive components (MIT/BSD-2/Apache-2.0/MPL-2.0/Zlib/Public-Domain) get attribution-only.
- **Alternatives:** Bundle the actual upstream source tarballs in the SLC image.
- **Rationale:** GPLv2 §3(b) and LGPL-2.1 permit a 3-year written offer for unmodified upstream binaries. All bundled components are unmodified upstream, shipped as separate files; bundling source would bloat the SLC for no practical benefit. libstdc++/libgcc carry the GCC Runtime Library Exception, so no copyleft propagates to `exaudfclient` or user UDFs.
- **Promotes to ADR:** yes

### [3] Cover the Alpine distribution tarball only; leave Dockerfile.debian untouched

- **Decision:** Add the notice bundle to `Dockerfile.alpine`'s `runtime` stage only. Do not modify `Dockerfile.debian`.
- **Alternatives:** Produce a parallel notice bundle for the Debian image.
- **Rationale:** Only `Dockerfile.alpine` produces a distribution tarball (`lc-rs.tar.gz` via the `artifact` stage, uploaded by `scripts/install.sh`). `Dockerfile.debian` produces no tarball and is referenced by no script, test, or CI — it is a dev/test image only, so it redistributes nothing.
- **Promotes to ADR:** yes

### [4] Place the COPY in the runtime stage, not the packager stage

- **Decision:** Add the notice files in the `runtime` stage so the `packager` stage's `tar` of the rootfs carries them into `lc-rs.tar.gz` without touching the tar exclude list.
- **Alternatives:** Inject the files in the `packager` stage after the rootfs copy.
- **Rationale:** The `packager` stage tars the runtime rootfs into `/slc`; files present in `runtime` flow through automatically. Adding them in `runtime` keeps the change to a single COPY and avoids editing the exclude list.
- **Promotes to ADR:** no

### [5] Patch bump 0.13.0 → 0.13.1

- **Decision:** Bump the workspace version to 0.13.1; leave `exarrow-rs` and all other dependencies unchanged.
- **Alternatives:** Fold the change into the in-flight 0.13.0; minor bump to 0.14.0.
- **Rationale:** v0.13.0 is being tagged/released independently right now; this is a notice-only change with no API surface, so a patch bump is the correct SemVer step. (Interview decision.)
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
