# Tasks: add-name-resolution

## Phase 2: Implementation (Group A — parallel)
- [x] 2.1 Stage name-resolution config into Dockerfile.alpine
- [x] 2.2 Add name-resolution DNS UDF crate (resolves www.exasol.com, no connect-back), workspace member, harness SLC wiring; remove CB_SELF_HOST / connect_back_hostname_address helper

## Phase 2: Implementation (Group B — after A)
- [x] 2.3 Runtime confirmation spike: build patched image, run DNS-resolution e2e test (www.exasol.com → valid IP) against 2025.1.11

## Phase 2: Implementation (Group C — after B)
- [x] 2.4 Add name_resolution_resolves_external_hostname integration scenario (assert emitted value parses as valid IpAddr)

## Phase 2: Implementation (Group D — after C)
- [x] 2.5 Defensive Debian check and record result
- [x] 2.6 Update CLAUDE.md with Alpine name-resolution note

## Phase 2: Implementation (Group E — cleanup, after verification)
- [x] 2.7 Remove `setup_dns()` from `crates/exaudfclient/src/main.rs` (confirmed dead code: SLC filesystem is read-only at sandbox runtime, so `std::fs::write()` silently fails; the function never does anything); update CLAUDE.md to remove the belt-and-suspenders reference
- [x] 2.8 Revert Dockerfile.alpine nsswitch.conf and host.conf additions (Task 2.1 found redundant — Alpine base already ships nsswitch.conf with `hosts: files dns`; neither file affects DNS resolution)
- [x] 2.9 Extract symlink post-processing into a shared packaging tool (script or `cargo exaudf package`) so IT harness and production both use the same path — `patch_resolver_symlinks()` moves out of `crates/it/src/lib.rs` into the shared tool; IT calls the tool rather than embedding the logic

## Phase 3: Verification
- [x] 3.1 Build: cargo build --release exits 0
- [x] 3.2 Build image: docker build Dockerfile.alpine exits 0; /etc/nsswitch.conf present
- [x] 3.3 Test: cargo test (unit/integration) exits 0
- [x] 3.4 e2e gate: EXASOL_VERSION=2025.1.11 scenario name_resolution_resolves_external_hostname PASSES (emits valid IP) — CONFIRMED 2026-06-15
- [x] 3.5 Lint: cargo clippy exits 0
- [x] 3.6 Format: cargo fmt --check no changes
