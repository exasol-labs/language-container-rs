# Decision Log: add-name-resolution

Date: 2026-06-15

## Interview

**Q:** Which container image(s) should the fix target?
**A:** Alpine only. `Dockerfile.alpine` is the active/primary slim image and the one missing `/etc/nsswitch.conf`. The `debian:12-slim` base already ships nsswitch.conf via base-files, so Debian likely already resolves names. The plan may note a defensive check of Debian, but the deliverable targets Alpine.

**Q:** How should `/etc/resolv.conf` be handled?
**A:** Originally deferred to a runtime spike. **Now DECIDED** by inspecting the real shipped built-in Python3 SLC (`exasol/docker-db:2025.1.11`, flavor `standard-EXASOL-all-python-3.10`, SLC v11.1.1): the Python3 SLC ships `/etc/resolv.conf` as a **symlink → `/conf/resolv.conf`**, and the DB injects the actual resolver config at `/conf/resolv.conf` inside the sandbox at runtime. The Alpine image must recreate the same symlink. Do NOT hardcode a nameserver. The remaining spike is a runtime confirmation only (does `/conf/resolv.conf` populate and does `getaddrinfo` then succeed for our SLC), not a discovery of the mechanism.

**Q:** How should the fix be verified?
**A:** An integration test that resolves a real hostname inside the sandbox end-to-end. **REVISED 2026-06-15:** the verification is a DNS-resolution check only — a `SET SCRIPT` UDF resolves the external hostname `www.exasol.com` via `getaddrinfo`/`ToSocketAddrs` and emits the resolved IP; the harness asserts the result is non-empty and parses as a valid `IpAddr`. The UDF opens NO connect-back session. This isolates the image's DNS configuration from any connect-back behaviour and proves the fix works at UDF runtime, not just that a config file exists in the image. (The earlier design — a connect-back-via-hostname UDF asserting the resolved address equals the node eth0 IP and querying `SELECT 42` — was dropped as unnecessarily coupled; see revised Design Decision [6].)

## Design Decisions

### [1] Root cause: missing nsswitch.conf despite bundled NSS modules

- **Decision:** Diagnose the failure as the absence of `/etc/nsswitch.conf` in the Alpine image. The image bundles `libnss_files.so.2` and `libnss_dns.so.2` into `/glibc-rt/` (`Dockerfile.alpine:37-39`) and copies them wholesale into the runtime stage (`:59`), but creates no `nsswitch.conf`, so glibc's NSS dispatcher has no switch config (`hosts: files dns`) and never reliably consults the bundled `libnss_dns` module → `getaddrinfo` fails.
- **Alternatives:** Suspected musl resolution, missing resolv.conf alone, or a connect-back code defect — all rejected by codebase exploration (`connect_back.rs build_dsn()` uses pre-resolved IPs; the binary is glibc-linked; modules are present but unconsulted).
- **Rationale:** The NSS modules being present but ineffective points squarely at the missing switch config.
- **Promotes to ADR:** yes

### [2] glibc-NSS approach, not musl

- **Decision:** Fix via glibc `nsswitch.conf` + bundled glibc NSS modules, not musl resolver config.
- **Alternatives:** Configure musl name resolution in the Alpine runtime.
- **Rationale:** `exaudfclient` is glibc-linked and runs inside the Debian/glibc Exasol host's network namespace after BucketFS extraction (CLAUDE.md "SLC container image" spike; `specs/container/slim-image`). Alpine is only a packaging envelope; musl resolution is never exercised at UDF runtime.
- **Promotes to ADR:** yes

### [3] Stage config into `/glibc-rt/` reusing the ld.so.conf pattern

- **Decision:** Write `nsswitch.conf` (and spike-determined host.conf/hosts/resolv.conf) into `/glibc-rt/etc/` in the builder `RUN` layer that already stages `ld.so.conf` (`Dockerfile.alpine:40-45`), letting the existing wholesale `COPY --from=builder /glibc-rt/ /` carry it.
- **Alternatives:** A separate `COPY`/`RUN` in the Alpine runtime stage.
- **Rationale:** Reuses an established, proven pattern; adds no new image layers; single source of truth for staged `/etc`.
- **Promotes to ADR:** no

### [4] Why official Exasol containers do not hit this bug

- **Decision:** Record that official `exasol/script-languages` flavors build on a full distro base and `COPY --from={{build_deps}} /etc /etc`, inheriting the distro's stock `/etc/nsswitch.conf` (`hosts: files dns`). A repo-wide grep of both official repos found no explicit nsswitch/resolv/hosts references — they get resolution "for free" and rely on the DB runtime to supply `/etc/resolv.conf`. **Corroborated by primary evidence**: inspecting the real shipped built-in Python3 SLC (`2025.1.11`, see Design Decision [7]) confirms the SLC's own `/etc` carries `nsswitch.conf` (`hosts: files dns`), `host.conf`, an empty `hosts`, and a `resolv.conf` symlink → `/conf/resolv.conf` — exactly what a full-distro base ships. The full-distro inheritance is the background mechanism; the shipped-SLC inspection is the ground truth. (PR exasol/script-languages#30 is a separate pyodbc-IPv6 issue, not the missing-nsswitch root cause; worth a one-line note that Exasol prefers IPv4 for connect-back.)
- **Alternatives:** N/A (context capture).
- **Rationale:** Explains why a slim-from-scratch image must add what a full-distro base provides implicitly, and why hardcoding DNS is wrong.
- **Promotes to ADR:** yes

### [5] resolv.conf strategy: symlink → /conf/resolv.conf (verified, not spike-dependent)

- **Decision:** Ship `/etc/resolv.conf` as a symlink → `/conf/resolv.conf` in the Alpine image (staged as `/glibc-rt/etc/resolv.conf -> /conf/resolv.conf`), mirroring the real shipped Python3 SLC. The DB injects the actual resolver config at `/conf/resolv.conf` inside the sandbox at runtime. Do NOT hardcode an upstream nameserver, ship an empty placeholder, or rely on no file at all. The wholesale `COPY --from=builder /glibc-rt/ /` must preserve the symlink (not dereference it).
- **Alternatives:** Hardcode `nameserver 8.8.8.8`; ship an empty resolv.conf; ship no resolv.conf and rely on DB injection at the default path; defer to a discovery spike (the former plan).
- **Rationale:** Direct inspection of the real shipped built-in Python3 SLC (Design Decision [7]) shows the proven mechanism is a symlink to `/conf/resolv.conf`. Hardcoding breaks DB-managed DNS; an empty/absent file would not pick up the DB-injected resolver. The remaining Task 1 spike is a runtime confirmation (does `/conf/resolv.conf` populate for our SLC, does `getaddrinfo` then succeed), not a discovery of the strategy.
- **Promotes to ADR:** yes

### [7] Required `/etc` file set verified against the real shipped Python3 SLC

- **Decision:** Mirror the exact name-resolution `/etc` file set observed in the real shipped built-in Python3 SLC (`exasol/docker-db:2025.1.11`, flavor `standard-EXASOL-all-python-3.10`, SLC v11.1.1, inspected via `docker run --rm --entrypoint /bin/bash`, DB not running): `/etc/nsswitch.conf` with `hosts:          files dns` (regular file), `/etc/host.conf` (`order hosts,bind` / `multi on`, regular file), `/etc/hosts` as a **symlink → `/conf/hosts`**, and `/etc/resolv.conf` as a **symlink → `/conf/resolv.conf`**. (`/etc/hostname` is an empty regular file — minor, optional to ship.) The SLC is glibc-based and already ships `libnss_files.so.2` + `libnss_dns.so.2` — the same two modules our Alpine image bundles — which suffice for `hosts:` resolution.
- **CORRECTION (2026-06-15):** `/etc/hosts` was previously recorded here as an empty regular file. Re-inspection with `ls -la` / `readlink` on the real SLC shows it is a **symlink → `/conf/hosts`**, exactly like `resolv.conf`. It only appeared empty before because the symlink target `/conf/hosts` does not exist in the static image — the DB populates `/conf/` at runtime. The fix must stage `ln -s /conf/hosts /glibc-rt/etc/hosts` (not an empty file), and the wholesale `COPY --from=builder /glibc-rt/ /` must preserve both `hosts` and `resolv.conf` symlinks.
- **Pattern (rationale for the split):** static resolver POLICY ships as regular files in the image (`nsswitch.conf`, `host.conf`); runtime-variable DATA symlinks into `/conf/` (`hosts → /conf/hosts`, `resolv.conf → /conf/resolv.conf`), which the Exasol DB populates per-deployment inside the sandbox at runtime. This policy(static)-vs-data(/conf symlink) split is the consistent mechanism and explains why both `hosts` and `resolv.conf` are symlinks rather than baked-in files.
- **Alternatives:** Derive the file set from a runtime discovery spike (the former plan); guess the file set from GitHub source of `exasol/script-languages`; ship `/etc/hosts` as an empty regular file (the prior, incorrect finding).
- **Rationale:** Inspecting the actually shipped SLC is primary, ground-truth evidence — strictly stronger than reasoning from GitHub source or a guessed-then-spiked file set. It collapses the largest open risk (resolv.conf strategy) into a decided fact and reduces Task 1 to a confirmation. The official build inherits these files implicitly from its full-distro base; our slim Alpine image strips `/etc` and so must recreate them explicitly.
- **Promotes to ADR:** yes

### [6] Verification is DNS-resolution-only — no connect-back (REVISED 2026-06-15)

- **Decision:** The e2e verification UDF resolves the **external hostname `www.exasol.com`** via `std::net::ToSocketAddrs`/`getaddrinfo` and emits the first resolved IP as a string; the harness asserts the emitted value is non-empty and parses as a valid `std::net::IpAddr` (IPv4 or IPv6). The UDF is a `SET SCRIPT ... EMITS (...)` but opens NO connect-back session, reads no CONNECTION object, and makes no assertion about the node eth0 IP. The scenario lives in `container/slim-image` (a runtime verification of the image config), not in `integration/connect-back`.
- **Supersedes (prior decision):** the earlier design required a connect-back-via-hostname UDF whose `CB_SELF_HOST` CONNECTION address used a hostname resolving to `<container-eth0-ip>` (never loopback), opening a connect-back session, querying `SELECT 42`, and asserting the resolved address equalled `container_inner_ip()`. That coupled the DNS-config test to connect-back transport, Serializable-isolation invariants, the SCALAR-vs-SET-SCRIPT crash rule, and a harness-controlled hostname→eth0-IP mapping (`connect_back_hostname_address()`, `CB_SELF_HOST`).
- **Alternatives:** Keep the connect-back-via-hostname verification; resolve a harness-controlled `/etc/hosts` alias instead of an external DNS name; assert the resolved IP equals the node eth0 IP.
- **Rationale:** Resolving an external public hostname end-to-end through `libnss_dns` + the DB-injected `/etc/resolv.conf -> /conf/resolv.conf` is the most direct proof that glibc name resolution works at UDF runtime, and it removes all the connect-back coupling (no loopback/eth0-IP assertion, no CONNECTION object, no Serializable-isolation concern, no harness hostname-mapping helper). A name-resolution failure surfaces as a `getaddrinfo` error rather than being entangled with connect-back SIGABRT modes, so a misconfiguration cannot masquerade as something else.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
