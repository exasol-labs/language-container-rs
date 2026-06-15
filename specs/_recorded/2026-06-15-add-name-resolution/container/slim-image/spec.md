# Feature: slim-image

Packages the `exaudfclient` binary into a slim Alpine-based SLC Docker image that Exasol can register as a `localzmq+protobuf` language container. This delta adds glibc name-resolution config so hostnames resolve at UDF runtime.

## Background

The Alpine image bundles the glibc NSS modules `libnss_files.so.2` and `libnss_dns.so.2` into `/glibc-rt/` and copies that tree wholesale into the runtime stage, but creates no `/etc/nsswitch.conf`. Without a switch config (`hosts: files dns`), glibc never reliably consults the bundled `libnss_dns` module, so hostname resolution fails inside the UDF sandbox. The `exaudfclient` binary is glibc-linked and runs in the Debian/glibc Exasol host's network namespace after BucketFS extraction, so the glibc-NSS fix (not musl) is correct.

The required set of files is no longer spike-dependent: inspection of the **real shipped built-in Python3 SLC** (`exasol/docker-db:2025.1.11`, flavor `standard-EXASOL-all-python-3.10`, SLC v11.1.1) shows the Python3 SLC ships in its own `/etc`: `nsswitch.conf` with `hosts:          files dns`, `host.conf` containing `order hosts,bind` / `multi on`, `/etc/hosts` as a **symlink → `/conf/hosts`**, and `/etc/resolv.conf` as a **symlink → `/conf/resolv.conf`** (the DB injects both the real resolver config and the hosts data at `/conf/` inside the sandbox at runtime; no nameserver and no host entries are hardcoded). The official build inherits these files implicitly from its full-distro base; the slim Alpine image strips `/etc` and so must recreate them explicitly. The fix stages this exact set into `/glibc-rt/etc/`, reusing the existing `ld.so.conf` staging pattern so the one wholesale `COPY --from=builder /glibc-rt/ /` carries it.

**The consistent pattern:** static resolver POLICY ships as regular files in the image (`nsswitch.conf`, `host.conf`); runtime-variable DATA symlinks into `/conf/` (`hosts → /conf/hosts`, `resolv.conf → /conf/resolv.conf`), which the Exasol DB populates per-deployment inside the sandbox at runtime. The wholesale `COPY --from=builder /glibc-rt/ /` must preserve (not dereference) both symlinks.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Builder toolchain and glibc runtime

* *GIVEN* the Dockerfile builder stage `FROM rust:1.91-bookworm`
* *WHEN* the image is built
* *THEN* the builder MUST install `protobuf-compiler` and `pkg-config` but NOT `libzmq3-dev`
* *AND* zmq MUST be statically linked via `zeromq-src`
* *AND* the glibc runtime libs MUST be collected via `cp -L` into `/glibc-rt/` and staged into the runtime image
* *AND* the staged `/glibc-rt/` tree MUST also carry the name-resolution config so that glibc's NSS dispatcher can consult the bundled `libnss_files.so.2` and `libnss_dns.so.2` modules at UDF runtime — at minimum `/glibc-rt/etc/nsswitch.conf` declaring `hosts: files dns`, mirroring the existing `ld.so.conf` staging pattern in the same `RUN` layer
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Alpine runtime image carries glibc name-resolution config

* *GIVEN* the `slc-rs-slim:dev` image built from `Dockerfile.alpine`
* *WHEN* the runtime stage filesystem is inspected after the wholesale `COPY --from=builder /glibc-rt/ /`
* *THEN* the image MUST contain `/etc/nsswitch.conf` whose `hosts:` line lists `files` then `dns` (so glibc resolves `/etc/hosts` entries first, then falls back to DNS via the bundled `libnss_dns.so.2`), and the bundled `libnss_files.so.2` / `libnss_dns.so.2` modules MUST remain present at the glibc-expected path so the dispatcher can `dlopen` them
* *AND* the image MUST additionally provide `/etc/host.conf` containing `order hosts,bind` and `multi on` (a regular file — static resolver policy), `/etc/hosts` as a symlink to `/conf/hosts`, and `/etc/resolv.conf` as a symlink to `/conf/resolv.conf` (both symlinks — runtime-variable data), mirroring the real shipped built-in Python3 SLC; the wholesale `COPY --from=builder /glibc-rt/ /` MUST preserve (not dereference) both the `hosts` and `resolv.conf` symlinks; the image MUST NOT hardcode a specific upstream nameserver nor any host entries, because the DB injects the actual resolver config at `/conf/resolv.conf` and the hosts data at `/conf/hosts` inside the sandbox at runtime
* *AND* this change MUST NOT regress the existing scenarios — the binary path, language registration file, slimness, and db-roundtrip suite MUST all still hold
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Alpine runtime image resolves external hostnames at UDF runtime

* *GIVEN* a registered slim SLC session (built from the patched `Dockerfile.alpine`) and a deployed DNS-resolution UDF (`SET SCRIPT ... EMITS (resolved_addr VARCHAR(64))`, never `SCALAR`) that resolves the external hostname `www.exasol.com` via `std::net::ToSocketAddrs` (glibc `getaddrinfo`) and emits the first resolved IP as a string
* *AND* the UDF MUST NOT open any connect-back session — it performs name resolution only, exercising the staged `/etc/nsswitch.conf` (`hosts: files dns`) plus the bundled `libnss_files.so.2` / `libnss_dns.so.2` modules and the DB-injected `/etc/resolv.conf -> /conf/resolv.conf`
* *WHEN* the UDF is invoked over a live `exasol/docker-db:<version>` container so `getaddrinfo("www.exasol.com")` runs inside the real UDF sandbox
* *THEN* resolution MUST succeed and the UDF MUST emit a non-empty result, proving glibc name resolution works at UDF runtime — not merely that a config file exists in the image
* *AND* the harness MUST assert the emitted value parses as a valid IPv4 or IPv6 address (`std::net::IpAddr::from_str`) and is non-empty; a name-resolution failure MUST fail the test rather than be downgraded or skipped
* *AND* the harness MUST assert this as a hard assertion on every version in the matrix (`2025.1`, `2025.2`, `2026.1`); there is NO severity branch and NO unconditional skip
<!-- /DELTA:NEW -->
