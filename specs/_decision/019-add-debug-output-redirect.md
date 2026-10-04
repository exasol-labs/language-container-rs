# Decisions: add-debug-output-redirect

## ADR: Configure UDF verbosity via a `%udf_debug_level` directive, not an env var

**ID:** udf-debug-level-directive
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

A UDF author needs to raise SLC tracing verbosity without touching the cluster's process environment. The author controls `CREATE SCRIPT` text, which arrives in `source_code` after the handshake, while the subscriber is installed in `main()` before it.

### Decision

Verbosity is set by a `%udf_debug_level debug|info|warn|error` directive in the script source, parsed from the handshake `source_code` field like `%udf_object`. Absent or unrecognised values resolve to `info`.

### Options Considered

| Option | Verdict |
|--------|---------|
| `%udf_debug_level` directive in `CREATE SCRIPT` source | ✓ Chosen |
| `RUST_LOG` env var read at `main()` init | ✗ Read before the handshake; no per-script level |
| `std::env::set_var("RUST_LOG", ...)` before `init()` | ✗ Has no effect after `init()` |

### Consequences

Lines logged before the handshake always use the default level `info`.

## ADR: Output redirect is the database's job (fd-level dup2), not an SLC-managed TCP sink

**ID:** output-redirect-is-database-job-fd-dup2
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

The database already implements output redirect in `Engine/src/exscript/pluggable/zmqinternal.cc`. It reads `SET SESSION SCRIPT OUTPUT ADDRESS`, opens a TCP socket, and `posix_spawn_file_actions_adddup2`s it onto the child's fd 1 and fd 2 before spawning `nschroot` → `exaudfclient`.

### Decision

The SLC relies on `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` and writes diagnostics to stderr. It has no `%udf_debug_output` directive, no SLC TCP connection and no crash-report subsystem. The `runtime/crash-report` spec does not exist.

### Options Considered

| Option | Verdict |
|--------|---------|
| Database fd-level `SET SESSION SCRIPT OUTPUT ADDRESS` | ✓ Chosen: also captures startup failures and hard crashes |
| SLC-managed `%udf_debug_output` with a post-handshake TCP layer | ✗ Duplicates a database feature; misses pre-handshake crashes |
| Alloc/signal handlers writing crash reports to BucketFS | ✗ The fd-2 redirect already delivers panics and aborts |

### Consequences

The SLC has no TCP connection management, alloc-error hook, signal handler or BucketFS write path.

## ADR: UDF logging via `udf_log!` + `UdfContext::debug_level`, writing to stderr

**ID:** udf-logging-via-udf-log-macro-stderr
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

The UDF `.so` statically links its own `tracing`, so its dispatcher is separate from the host's, and sharing it across the `dlopen` boundary relies on the static-identity pattern this project bans. UDF authors still need level-filtered lines in the stderr stream the database redirect captures.

### Decision

`UdfContext` has a default `fn debug_level(&self) -> tracing::Level { tracing::Level::INFO }`. The host `HostContextBridge` returns the session's resolved level. The `udf_log!(ctx, level, ...)` macro writes to stderr only when `ctx.debug_level()` permits the level.

### Options Considered

| Option | Verdict |
|--------|---------|
| `udf_log!` writing to stderr, gated by `ctx.debug_level()` | ✓ Chosen |
| Share the runtime's `tracing` dispatcher across the `dlopen` boundary | ✗ Banned static-identity pattern; breaks when `tracing` instances differ |

### Consequences

Existing UDFs compile unchanged because `debug_level()` has a default body. The `dyn UdfContext` vtable changes, so a stale `.so` is rejected with `AbiMismatch`.
