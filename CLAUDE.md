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

- Both `SCALAR` and `SET` scripts support connect-back; choose whichever UDF type fits the logic.
- Address must be `<container-eth0-ip>:8563` via `ctx.cluster_ip()`; never `127.0.0.1` or the Docker host gateway (both → SIGABRT). `cluster_ip()` reads the first non-loopback IPv4 via `getifaddrs`; tests get it from `container_inner_ip()`.
- Connect-back is a plain SQL login using CONNECTION-object credentials, running in its own independent transaction. Read-only is always safe; write-back must not write-write/schema-conflict with the invoking query (else WAIT FOR COMMIT → deadlock abort, Part:40 SIGABRT ~T+11s).
- Transport (native binary vs WebSocket) is irrelevant — UDF type is the differentiator.

## exaudfclient lifecycle

- End `main()` with `std::process::exit(0)` — never return normally. A normal return joins the connect-back Tokio runtime threads, delaying exit 10+s → Part:40 `SIGABRT` ~T+11s.

## Emit buffering and wire limits

- `MT_EMIT` messages have a wire limit of **exactly 4,000,000 bytes** — `EMIT_BUFFER_LIMIT_BYTES = 4_000_000`, matching the reference C++ SLC's `SWIG_MAX_VAR_DATASIZE = 4_000_000`. This is 4 *million* bytes, NOT 4 MiB (4,194,304). The C++ launcher flushes after every row that crosses it.
- `ctx.emit` must **not** send a message per call. Buffer rows and flush to `MT_EMIT` only when the byte estimate reaches 4,000,000 bytes.
- **Always flush at end of `run()`** — even if the threshold was not reached. The architect rule: "beim buffern ist auch wichtig, das man flushed, wenn die Run Methode durch ist".
- A single row can be up to 2 GB — this limit cannot be avoided. A row that alone exceeds the 4,000,000-byte threshold must still be sent as a single-row `MT_EMIT` (no way to split it).
- `EmitBuffer` must maintain a running byte-size estimate updated on each `push`, not recomputed on flush.

## Connect-back streaming

- `ExaConnection::query` is **collect-all** — the entire result set materialises in memory as `Vec<Vec<Value>>`. Use it only for small, bounded result sets.
- For table-scale reads, use the streaming API: fetch Arrow batches one at a time, convert each batch → `Vec<Value>` chunk, yield/callback to the caller, then **drop the batch before fetching the next one**. The architect rule: "du musst resultset in batches lesen und dann gleich emitten".
- Never accumulate all `RecordBatch`es before converting — that creates two in-memory copies (Arrow + Value) of the entire result simultaneously.
- The `ExaConnection` trait (SDK/FFI boundary) must remain **Arrow-free**: only `Vec<Value>` chunks cross the `.so` boundary; Arrow `TypeId` is not stable across dynamic library boundaries.
- The natural consumer pattern is emit-as-you-read: `conn.query_for_each(sql, |row| ctx.emit(&row))` — read a chunk, emit it, discard it, repeat.

## Misc

- Keep the three "connection" concepts distinct: Exasol CONNECTION object (credential store) vs exarrow-rs session (the connect-back act) vs cluster node IP (`ctx.cluster_ip()`).
- The ZMQ control channel is DB-chosen (`ipc://` single-node, `tcp://` multi-node), not settable via `SCRIPT_LANGUAGES`, and has no effect on connect-back (always TCP to :8563).
- Alpine vs Debian SLC image makes no difference to connect-back (spike 2026-06-09).
