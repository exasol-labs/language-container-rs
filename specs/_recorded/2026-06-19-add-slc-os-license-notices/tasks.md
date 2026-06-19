# Tasks: add-slc-os-license-notices

Approach (revised per user direction): the OS-license manifest is **generated**
by `cargo about` from a committed `dist/` boilerplate; the generated
`THIRD-PARTY-OS-LICENSES.md` is git-ignored (embeds texts) and produced before
the Docker build (locally via `scripts/install.sh` / `scripts/ci-it-local.sh`,
and in the CI `build-slc` job). No verbatim license text is committed.

## Group A — dist/ generator boilerplate + version bump
- [x] A.1 `dist/os-attribution/{Cargo.toml,src/lib.rs}` — synthetic crate; `license` expr = OS license list
- [x] A.2 `dist/about-os.toml` (accepted list), `dist/os-licenses.hbs` (apk table + glibc section + 3-year written offer + license-text loop), `dist/generate-os-licenses.sh` (cargo about + append GCC exception), `dist/.gitignore` (ignore generated manifest) [expert]
- [x] A.3 Bump version 0.13.0 → 0.13.1 (workspace.package + exasol-udf-sdk dep); `cargo check` regenerates Cargo.lock

## Group B — wiring
- [x] B.1 `Dockerfile.alpine` runtime stage COPYs `dist/THIRD-PARTY-OS-LICENSES.md` into `/exaudf`
- [x] B.2 `scripts/install.sh` + `scripts/ci-it-local.sh` run the generator before `docker build`
- [x] B.3 CI `build-slc` job installs cargo-about + runs the generator before `docker build`

## Group C — verification
- [x] C.1 Generator produces manifest: apk table, glibc section, written offer, 8 license texts + GCC exception (1160 lines)
- [x] C.2 `cargo fmt --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` 0 warnings; `cargo test` 46 ok / 0 failed (fixtures rebuilt for 0.13.1 fingerprint)
- [x] C.3 Tarball carries `exaudf/THIRD-PARTY-OS-LICENSES.md` + `exaudf/LICENSE` + `exaudf/THIRD-PARTY-LICENSES.md`
- [x] C.4 db-roundtrip E2E against live Docker: 20/20 scenarios pass (incl. timestamp + connect_back_dml)
