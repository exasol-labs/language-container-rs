# Feature: debug-output

A UDF author developing a Rust UDF CAN watch the SLC's runtime diagnostics live and control their verbosity, without changing or rebuilding the precompiled `.so`. The live stream itself is provided by the **database**, not the SLC: the Exasol session command `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` makes the engine redirect the spawned UDF process's stdout and stderr to a TCP listener at the OS file-descriptor level *before* the process starts, so every line the process writes to stderr — runtime `tracing`, startup errors, and even a hard native crash before any Rust code runs — reaches the listener. On top of that DB-provided surface, this feature gives the author two SLC-side controls: a `%udf_debug_level` `CREATE SCRIPT` directive that tunes how verbose the runtime's `tracing` output is, and an SDK logging surface (`udf_log!` / `UdfContext::debug_level`) so UDF code can emit its own level-filtered lines to the same stderr stream. At `debug` level the runtime additionally writes memory and emit-buffer telemetry so an operator can watch resource growth live.

## Background

* The runtime writes all diagnostics to stderr through the `tracing` crate; Exasol captures the UDF process's stderr as the UDF log, and `SET SESSION SCRIPT OUTPUT ADDRESS` redirects that same stderr to a TCP listener.
* The redirect is performed by the database engine via `posix_spawn` file-descriptor dup2 onto a socket it opens before spawning the UDF process; the SLC manages no TCP connection of its own and needs no code to participate in it.
* The runtime parses `%udf_*` directives from the `source_code` field of the handshake metadata, the same place `%udf_object` is resolved. The `source_code` is only available after the handshake, so the verbosity level applies from the moment the runtime resolves it onward; lines emitted during early `main()`/handshake use the process-default level.
* The SDK logging surface writes to the process's stderr (file descriptor 2). It does not create or depend on a `.so`-local `tracing` subscriber, dispatcher, or other global state.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Connect-back diagnostics use the gated tracing channel and write no file

* *GIVEN* a runtime built with the `connect-back` feature at any resolved debug level
* *WHEN* a UDF calls `query`, `query_for_each`, `execute` or `execute_batch`, or a connect-back session is dropped
* *THEN* every connect-back diagnostic line MUST be emitted through `tracing::debug!`, so the resolved `%udf_debug_level` gates it and it reaches the database's stderr redirect like every other runtime line
* *AND* the runtime MUST NOT create, open or append to any file for diagnostics: the unconditional `/tmp/cb_debug.txt` append MUST be removed, because a per-row lookup UDF paid an open, write and close syscall triple on every call and the staged container guarantees no writable diagnostic path
* *AND* the SQL text MUST be carried as a structured tracing field rather than a pre-formatted string, so a session at the default level performs no per-call formatting work
<!-- /DELTA:NEW -->
</content>
