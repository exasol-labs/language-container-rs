# Project Rules

**Spec-driven project using speq-skill.**

Project mission in: @specs/mission.md

## Exasol / tooling

- Use Exasol Docker images to run Integration Tests and E2E tests
- Use `exapump` for all Exasol interaction.
- DSNs must include `validateservercertificate=0` (self-signed Docker cert).

## CI (Ubuntu 24.04 runners)

- Run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` before `docker run` of the Exasol DB.
- Otherwise every UDF reports `Internal error: VM crashed` (SQL state 22002) — AppArmor strips `CAP_SYS_ADMIN` from `nschroot` even under `--privileged`. It is **not** memory/disk/kernel/glibc/UDF code. Green locally on Debian; confirm on the runner via `sudo dmesg` (`apparmor="DENIED" ... comm="nschroot" capability=21`).

## Connect-back

- Use `SET SCRIPT ... EMITS (...)` for any connect-back UDF; **never** `SCALAR` (SCALAR → SIGABRT mid-execution).
- Address must be `<container-eth0-ip>:8563` via `ctx.cluster_ip()`; never `127.0.0.1` or the Docker host gateway (both → SIGABRT). `cluster_ip()` reads the first non-loopback IPv4 via `getifaddrs`; tests get it from `container_inner_ip()`.
- Connect-back is a plain SQL login using CONNECTION-object credentials, running in its own independent transaction. Read-only is always safe; write-back must not write-write/schema-conflict with the invoking query (else WAIT FOR COMMIT → deadlock abort, Part:40 SIGABRT ~T+11s).
- Transport (native binary vs WebSocket) is irrelevant — UDF type is the differentiator.

## exaudfclient lifecycle

- End `main()` with `std::process::exit(0)` — never return normally. A normal return joins the connect-back Tokio runtime threads, delaying exit 10+s → Part:40 `SIGABRT` ~T+11s.

## Misc

- Keep the three "connection" concepts distinct: Exasol CONNECTION object (credential store) vs exarrow-rs session (the connect-back act) vs cluster node IP (`ctx.cluster_ip()`).
- The ZMQ control channel is DB-chosen (`ipc://` single-node, `tcp://` multi-node), not settable via `SCRIPT_LANGUAGES`, and has no effect on connect-back (always TCP to :8563).
- Alpine vs Debian SLC image makes no difference to connect-back (spike 2026-06-09).
