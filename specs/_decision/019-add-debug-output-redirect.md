# Decisions: add-debug-output-redirect

## ADR: Configure UDF verbosity via a `%udf_debug_level` directive, not an env var

**ID:** udf-debug-level-directive
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

A UDF author chasing a bug needs to raise the SLC's tracing verbosity to `debug` without touching the cluster's process environment. The `tracing-subscriber` subscriber is installed in `main()` before the handshake; the `source_code` field carrying per-script configuration arrives only after the handshake completes. The author controls `CREATE SCRIPT` text but not the SLC process environment inside a production cluster.

### Decision

Declare verbosity as a `%udf_debug_level debug|info|warn|error` directive in the script source, parsed from the `source_code` field of the handshake metadata — the same channel already used by `%udf_object`. The parser defaults to `info` for absent or unrecognised values.

### Options Considered

| Option | Verdict |
|--------|---------|
| `%udf_debug_level` directive in `CREATE SCRIPT` source | ✓ Chosen — the only knob reachable without touching the container's process env |
| Rely solely on `RUST_LOG` env var read at `main()` init | ✗ Rejected — read before the handshake; cannot carry a per-script level |
| `std::env::set_var("RUST_LOG", ...)` before `init()` | ✗ Rejected — env var must be set before `init()`; setting it after has no effect |

### Consequences

Any author who can write `CREATE SCRIPT` SQL can tune SLC verbosity without a container rebuild or environment variable change. The directive is parsed after the handshake, so early `main()`/handshake lines always use the process-default level (`info`).

## ADR: Apply post-handshake log level via `tracing_subscriber::reload` + `rebuild_interest_cache`

**ID:** post-handshake-log-level-tracing-reload
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

The `%udf_debug_level` directive is parsed only after the handshake, but the `tracing-subscriber` is already installed in `main()`. The plan originally specified `tracing::level_filters::LevelFilter::set_max_level` as the one-line mechanism to raise the global max level at runtime. During implementation it was found that `set_max_level` does not exist in `tracing 0.1` — the API is not part of the public surface of that version.

### Decision

Install a `tracing_subscriber::reload`-wrapped `EnvFilter` in `main()`. After parsing `%udf_debug_level` post-handshake, call `reload_handle.reload(new_filter)` followed by `tracing::callsite::rebuild_interest_cache()`, which propagates the new level to the callsite interest cache and updates the value returned by `LevelFilter::current()`. This is a one-time mutation (no further reloads); no new crate dependency is added (`tracing-subscriber` already uses `reload` internally and the feature is available). The `reload::Handle` is stored as a field on `Runtime`.

### Options Considered

| Option | Verdict |
|--------|---------|
| `tracing_subscriber::reload` handle + `rebuild_interest_cache()` | ✓ Chosen — works correctly in `tracing 0.1`; one mutation, no extra dependency |
| `tracing::level_filters::LevelFilter::set_max_level` | ✗ Rejected (does not exist) — this API is absent from `tracing 0.1`'s public surface |
| Reinstall the entire subscriber post-handshake | ✗ Rejected — `init()` panics if called twice; requires unsafe global state reset |

### Consequences

The user-facing behavior (one-time post-handshake global level change, no subscriber reinstall) is identical to what the plan specified. The mechanism is `reload` + `rebuild_interest_cache()` rather than the originally cited `set_max_level`. The `reload` feature of `tracing-subscriber` is used but no new crate dependency is introduced. Events before the handshake use the process-default level.

## ADR: Output redirect is the database's job (fd-level dup2), not an SLC-managed TCP sink

**ID:** output-redirect-is-database-job-fd-dup2
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

Previous planning iterations proposed an SLC-managed live-output mechanism: a `%udf_debug_output host:port` directive, a `tracing_subscriber::reload` TCP layer opened post-handshake, plus alloc-error and signal handlers writing a crash report to BucketFS on native abort. The architect corrected this: output redirect is a database function already implemented in `Engine/src/exscript/pluggable/zmqinternal.cc`. The engine reads `SET SESSION SCRIPT OUTPUT ADDRESS`, opens a TCP socket, and `posix_spawn_file_actions_adddup2`s it onto the child's fd 1 (stdout) and fd 2 (stderr) *before* spawning `nschroot` → `exaudfclient`.

### Decision

Rely entirely on the database's `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` mechanism. The SLC writes diagnostics to stderr and does nothing else to provide the live stream. No `%udf_debug_output` directive, no SLC TCP connection, no crash-report subsystem. The `runtime/crash-report` spec is deleted. This decision is recorded explicitly so future planners do not re-propose SLC-managed redirect.

### Options Considered

| Option | Verdict |
|--------|---------|
| Use the DB's existing fd-level `SET SESSION SCRIPT OUTPUT ADDRESS` redirect | ✓ Chosen — captures everything an SLC layer would, plus startup failures and hard crashes before any Rust code runs |
| SLC-managed `%udf_debug_output` + reload TCP layer post-handshake | ✗ Rejected — reimplements a working DB feature; cannot capture crashes that occur before the post-handshake layer is installed |
| Two-mechanism crash reporting (alloc/signal handlers, BucketFS PUT) | ✗ Rejected — the fd-2 redirect already delivers panics, aborts, and signal-time stderr to the listener; bespoke subsystem is redundant complexity |

### Consequences

The SLC is simpler: no TCP connection management, no alloc-error hook, no signal handler, no BucketFS write path. The `runtime/crash-report` spec is permanently removed. The DB redirect captures startup errors and hard native aborts that an SLC-side TCP layer installed post-handshake could never see.

## ADR: UDF logging via `udf_log!` + `UdfContext::debug_level`, writing to stderr

**ID:** udf-logging-via-udf-log-macro-stderr
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

UDF code runs inside a `dlopen`-loaded `.so`. The `.so` statically links its own copy of `tracing`; its global dispatcher is a separate static from the host's. Cross-`.so` dispatcher sharing relies on the fragile static-identity pattern this project bans (see `dispatch-loader` spec and ADR for the ABI fingerprint). Yet UDF authors need a way to emit level-filtered diagnostic lines that reach the same stderr stream the DB redirect captures.

### Decision

Add a default `fn debug_level(&self) -> tracing::Level { tracing::Level::INFO }` to `UdfContext`. The host `HostContextBridge` implementation returns the session's currently resolved level. Add a `udf_log!(ctx, level, ...)` macro that formats and writes to stderr only when `ctx.debug_level()` permits the level. Writing to fd 2 directly bypasses the cross-`.so` dispatcher problem and lands in exactly the stream the DB redirect captures.

### Options Considered

| Option | Verdict |
|--------|---------|
| `udf_log!` macro writing to stderr, gated by `ctx.debug_level()` | ✓ Chosen — trivially correct; lands in the DB-redirected stream; no cross-`.so` globals |
| Share the runtime's `tracing` dispatcher across the `dlopen` boundary | ✗ Rejected — cross-`.so` static-identity pattern; banned by this project; silently broken when host and `.so` are not compiled with the same `tracing` instance |

### Consequences

UDF authors can emit level-filtered diagnostic lines to stderr without a `.so`-local subscriber or global state. Existing UDFs compile unchanged because `debug_level()` has a default body. The `dyn UdfContext` vtable changes, so the SDK version bumps per project rules and the ABI fingerprint is regenerated; a stale `.so` is rejected with a clear `AbiMismatch` error.
