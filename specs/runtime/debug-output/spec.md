# Feature: debug-output

A UDF author developing a Rust UDF CAN watch the SLC's runtime diagnostics live and control their verbosity, without changing or rebuilding the precompiled `.so`. The live stream itself is provided by the **database**, not the SLC: the Exasol session command `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` makes the engine redirect the spawned UDF process's stdout and stderr to a TCP listener at the OS file-descriptor level *before* the process starts, so every line the process writes to stderr — runtime `tracing`, startup errors, and even a hard native crash before any Rust code runs — reaches the listener. On top of that DB-provided surface, this feature gives the author two SLC-side controls: a `%udf_debug_level` `CREATE SCRIPT` directive that tunes how verbose the runtime's `tracing` output is, and an SDK logging surface (`udf_log!` / `UdfContext::debug_level`) so UDF code can emit its own level-filtered lines to the same stderr stream. At `debug` level the runtime additionally writes memory and emit-buffer telemetry so an operator can watch resource growth live.

## Background

* The runtime writes all diagnostics to stderr through the `tracing` crate; Exasol captures the UDF process's stderr as the UDF log, and `SET SESSION SCRIPT OUTPUT ADDRESS` redirects that same stderr to a TCP listener.
* The redirect is performed by the database engine via `posix_spawn` file-descriptor dup2 onto a socket it opens before spawning the UDF process; the SLC manages no TCP connection of its own and needs no code to participate in it.
* The runtime parses `%udf_*` directives from the `source_code` field of the handshake metadata, the same place `%udf_object` is resolved. The `source_code` is only available after the handshake, so the verbosity level applies from the moment the runtime resolves it onward; lines emitted during early `main()`/handshake use the process-default level.
* The SDK logging surface writes to the process's stderr (file descriptor 2). It does not create or depend on a `.so`-local `tracing` subscriber, dispatcher, or other global state.

## Scenarios

### Scenario: The database redirect captures all UDF process output

* *GIVEN* an Exasol session that has run `SET SESSION SCRIPT OUTPUT ADDRESS 'devhost:5000'` with a listener accepting connections at `devhost:5000`
* *WHEN* the session runs a query that invokes a Rust UDF
* *THEN* the database MUST redirect the spawned UDF process's stdout and stderr to that listener before the process starts
* *AND* the listener MUST receive the runtime's `tracing` output written to stderr during the session
* *AND* the listener MUST receive output produced even when the UDF binary fails to start or aborts before any UDF code runs, because the redirect is established at the file-descriptor level prior to spawn
* *AND* the SLC MUST NOT itself open, manage, or tear down any TCP connection to provide this stream

### Scenario: Debug level directive sets the runtime verbosity

* *GIVEN* a script source containing `%udf_debug_level debug`
* *WHEN* the runtime parses the debug-level directive after the handshake
* *THEN* it MUST resolve the runtime tracing verbosity to `debug`
* *AND* a source with no `%udf_debug_level` directive MUST default the verbosity to `info`
* *AND* an unrecognised level value MUST default to `info` rather than failing the session

### Scenario: The resolved level changes the global max verbosity at runtime

* *GIVEN* a script source whose `%udf_debug_level` resolves to a level above the process default
* *WHEN* the runtime applies the resolved level after the handshake
* *THEN* it MUST update the process-global maximum tracing level so subsequent events at the resolved level are emitted
* *AND* it MUST NOT require reinstalling or replacing the already-installed stderr subscriber
* *AND* events emitted before the level is applied MUST continue to use the process-default level

### Scenario: UDF code log calls reach the stderr stream

* *GIVEN* UDF code that logs through the SDK logging surface (`udf_log!(ctx, debug, ...)`) during `run()`
* *WHEN* the UDF emits a log message whose level the session's resolved debug level permits
* *THEN* the message MUST be written to the UDF process's stderr, reaching whatever the database has redirected stderr to (the UDF log, or a `SET SESSION SCRIPT OUTPUT ADDRESS` listener)
* *AND* a message whose level is below the session's resolved debug level MUST NOT be written
* *AND* the UDF MUST NOT need its own `tracing` subscriber, dispatcher, or `.so`-local global state for the message to appear

### Scenario: The context exposes the resolved debug level to UDF code

* *GIVEN* a session whose `%udf_debug_level` has resolved to a level
* *WHEN* UDF code queries `UdfContext::debug_level()`
* *THEN* the host context MUST report the session's currently resolved level
* *AND* a context with no host-provided level MUST report a default of `info`

### Scenario: Every runtime line is tagged with its origin VM

* *GIVEN* a session emitting runtime tracing to stderr
* *WHEN* a tracing event is written
* *THEN* each emitted line MUST carry the origin `pid`, `node_id`, and `session_id`
* *AND* it SHOULD carry the `vm_id` when the handshake metadata surfaces one
* *AND* these fields MUST let an operator de-interleave lines from many shard VMs streaming to one listener

### Scenario: Runtime lines are flushed individually

* *GIVEN* the runtime writing tracing events to stderr
* *WHEN* the runtime writes a tracing event
* *THEN* the writer MUST NOT accumulate lines in a userspace block buffer
* *AND* the most recent line MUST therefore be observable downstream even if the process aborts immediately after the write

### Scenario: Memory and emit-buffer telemetry is emitted at debug level

* *GIVEN* a session whose resolved debug level is `debug`
* *WHEN* the runtime crosses an emit-path phase transition or a periodic checkpoint during a run
* *THEN* it MUST emit a debug event reporting the process RSS read from `/proc/self/statm`
* *AND* it MUST report the current emit-buffer byte estimate, the cumulative emitted bytes, and the row/batch counts
* *AND* this telemetry MUST NOT be emitted when the resolved level is above `debug`

### Scenario: The emit and flush path is instrumented

* *GIVEN* a session whose resolved debug level permits the emit-path events
* *WHEN* the runtime buffers rows and flushes an `MT_EMIT` message
* *THEN* it MUST emit tracing events around the emit-buffer push and the flush recording bytes buffered, bytes flushed, and the flush outcome
* *AND* these events MUST originate host-side without crossing the `.so` boundary
