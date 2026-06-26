# Decision Log: add-debug-output-redirect

Date: 2026-06-26

## Interview

**Q:** How should a UDF author watch SLC output live during development?
**A:** Through the database's existing session command `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'`. The engine opens a TCP socket to that address and `dup2`s it onto the spawned UDF process's stdout (fd 1) and stderr (fd 2) before `posix_spawn`, so everything the process writes to stderr — runtime `tracing`, startup errors, and even a hard crash before any Rust code runs — reaches the listener. Output redirect is a DB function, not an SLC function; the SLC manages no TCP connection.

**Q:** Then what does the SLC need to add?
**A:** Only verbosity control and a UDF-side logging surface. A `%udf_debug_level debug|info|warn|error` directive in `CREATE SCRIPT`, parsed from `source_code` (same channel as `%udf_object`), tunes the runtime's tracing verbosity. A `udf_log!` macro plus `UdfContext::debug_level()` lets UDF code emit its own level-filtered lines to the same stderr stream.

**Q:** The directive value is only known after the handshake, but the subscriber is installed in `main()` before it. How is the level applied?
**A:** Call `tracing::level_filters::LevelFilter::set_max_level(level)` after parsing the directive. It mutates the process-global max level filter in place — no `tracing_subscriber::reload`, no `Box<dyn Layer>`, no extra feature flag. Events before the level is applied use the process default.

**Q:** Earlier amendments proposed an SLC-managed TCP redirect (`%udf_debug_output`), a reloadable TCP `tracing` layer, and a two-mechanism crash-report subsystem (alloc/signal handlers, BucketFS PUT). Are these kept?
**A:** No — they are wrong and removed. The architect's correction: output redirect is a DB function. The DB's fd-level redirect already captures *everything* an SLC-managed TCP layer could and more (including a binary that "doesn't start and dies hard", which an SLC layer installed post-handshake can never see). The crash-report subsystem is therefore unnecessary: the fd-2 redirect already delivers panics, aborts, and signal-time stderr to the listener. The `runtime/crash-report` spec is deleted.

**Q:** What telemetry is still wanted?
**A:** Keep the cheap, decisive parts: RSS from `/proc/self/statm` plus emit-buffer byte estimate / cumulative bytes / row+batch counts, gated behind `debug` level (R3); tracing spans/events around the emit/flush path host-side (R4); per-VM line tagging with `pid`/`node_id`/`session_id` (and `vm_id` if parsed) so interleaved shard-VM lines de-multiplex (R5); and confirm the stderr writer flushes per line so the last line before an abort is not lost (R6).

**Q:** Does adding `UdfContext::debug_level()` to the trait break ABI?
**A:** It changes the `dyn UdfContext` vtable, so the SDK version bumps per project rules (every change bumps `[workspace.package].version`). The ABI fingerprint is `SDK_VERSION:RUSTC_HASH`, so the version bump regenerates it; no separate fingerprint work is needed. Because `debug_level()` has a default trait-method body, existing UDFs compile unchanged against the new SDK.

## Design Decisions

### [1] Configure verbosity via a `%udf_debug_level` directive, not an env var or `RUST_LOG`

- **Decision:** Declare verbosity as a `%udf_debug_level debug|info|warn|error` directive parsed from the handshake `source_code`, reusing the channel `%udf_object` already uses.
- **Alternatives:** (a) Rely solely on the `RUST_LOG` env var read at `main()` init; (b) `std::env::set_var("RUST_LOG", ...)` before `init()`.
- **Rationale:** The author controls `CREATE SCRIPT` text, not the SLC process environment inside the cluster, so a directive is the only knob reachable without touching the container. `RUST_LOG` is read in `main()` before the handshake delivers `source_code`, so it cannot carry a per-script level; setting the env var after `init()` has no effect on an already-installed subscriber.
- **Promotes to ADR:** yes

### [2] Apply the level with `LevelFilter::set_max_level`, not `tracing_subscriber::reload`

- **Decision:** After parsing `%udf_debug_level`, call `tracing::level_filters::LevelFilter::set_max_level(level)` to change the process-global maximum verbosity at runtime.
- **Alternatives:** Install a `reload`-wrapped layer in `main()` and swap its `EnvFilter` after the handshake.
- **Rationale:** `set_max_level` is a one-line stdlib-of-tracing call that needs no extra `tracing-subscriber` feature, no `reload::Handle` threaded into `Runtime`, and no `Box<dyn Layer>`. It is the laziest mechanism that satisfies the requirement (lower the global max level at runtime). `reload` exists for arbitrary layer reconfiguration we do not need here.
- **Promotes to ADR:** yes

### [3] Output redirect is the database's job (fd-level dup2), not an SLC-managed TCP sink

- **Decision:** Rely on the database's `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` mechanism, which redirects the UDF process's stdout/stderr at the OS file-descriptor level before spawn. The SLC writes diagnostics to stderr and otherwise does nothing to provide the live stream. No `%udf_debug_output` directive, no SLC TCP connection, no crash-report subsystem.
- **Alternatives:** An SLC-managed redirect: a `%udf_debug_output host:port` directive, a `tracing_subscriber::reload` TCP layer opened post-handshake, plus alloc-error/signal handlers and a BucketFS crash-report PUT to capture native aborts.
- **Rationale:** Architect correction — output redirect is a DB function. The DB opens the socket and `dup2`s it onto fd 1/fd 2 *before* `posix_spawn`, so it captures everything an SLC layer would *and* the cases an SLC layer fundamentally cannot: startup failures and hard crashes before the post-handshake layer would even exist. An SLC TCP sink and a bespoke crash-report subsystem are therefore redundant complexity reimplementing a working DB feature. Recorded explicitly so future planners do not re-propose the SLC-managed redirect.
- **Promotes to ADR:** yes

### [4] UDF logging via `udf_log!` + `UdfContext::debug_level`, writing to stderr

- **Decision:** Add a default `fn debug_level(&self) -> tracing::Level` to `UdfContext` (host bridge returns the resolved level, default returns `info`) and a `udf_log!(ctx, level, ...)` macro that writes to stderr when the context's level permits.
- **Alternatives:** Share the runtime's `tracing` dispatcher across the `dlopen` boundary so a UDF's `tracing::info!` reaches the host subscriber.
- **Rationale:** The UDF `.so` statically links its own `tracing`; its global dispatcher is a separate static, so cross-`.so` dispatcher sharing relies on the fragile cross-`.so` static-identity pattern this project bans. Writing to stderr (fd 2) is trivially correct and lands in exactly the stream the DB redirect captures. `debug_level()` is a default trait method, so existing UDFs compile unchanged; the vtable change is covered by the routine version bump and ABI fingerprint.
- **Promotes to ADR:** yes

## Review Findings

<!-- Populated by speq-implement after code review. -->
