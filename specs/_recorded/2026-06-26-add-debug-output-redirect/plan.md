# Plan: add-debug-output-redirect

## Summary

Give Rust SLC developers a live debug surface without an SLC-managed network connection: rely on the database's existing `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` fd-level stdout/stderr redirect for the stream, and add SLC-side a `%udf_debug_level` verbosity directive, a `udf_log!` SDK logging surface, per-VM line tagging, and `debug`-gated memory/emit-buffer telemetry on the suspect emit path.

## Design

### Context

A UDF author chasing a bug (e.g. an OOM native abort in the emit path on a 60M-row UDF) needs to watch the SLC's diagnostics live and turn up verbosity, without rebuilding the precompiled `.so`. The Python3 SLC offers `exa.redirect_output(host, port)` from the script body; the Rust model has no script body.

The decisive correction from the architect: **output redirect is a database function, not an SLC function.** In `Engine/src/exscript/pluggable/zmqinternal.cc` (`start_udf_process_via_posix_spawn`), the engine reads the session's `SCRIPT OUTPUT ADDRESS`, opens a TCP socket to it, and `posix_spawn_file_actions_adddup2`s that socket onto the child's fd 1 (stdout) and fd 2 (stderr) *before* spawning `nschroot` → `exaudfclient`. Because the redirect is wired before the process starts, it captures **everything** the process writes to stderr — runtime `tracing`, startup errors, and even a binary that dies hard before any Rust code runs. The SLC needs no TCP code, no reload layer, and no crash-report subsystem to benefit: it only has to write to stderr.

What remains for the SLC is verbosity control and a UDF-side logging surface, plus making the emit path observable.

- **Goals** — A `%udf_debug_level` directive that tunes runtime tracing verbosity at runtime; a `udf_log!` / `UdfContext::debug_level` SDK surface so UDF code emits level-filtered lines to stderr; per-VM line tagging to de-interleave multi-node streams; `debug`-gated RSS + emit-buffer telemetry and emit/flush-path instrumentation; documentation of `SET SESSION SCRIPT OUTPUT ADDRESS` as the redirect mechanism.
- **Non-Goals** — Any SLC-managed TCP connection or `%udf_debug_output` directive (the DB does this); a `tracing_subscriber::reload` TCP layer; a crash-report subsystem (alloc-error/signal handlers, BucketFS PUT) — the DB's fd-2 redirect already delivers panic, abort, and signal-time stderr to the listener; redirecting UDF `println!`/stdout at the fd level (the DB already redirects fd 1).

### Decision

Keep the process-global stderr `tracing` subscriber installed in `main()`. After the handshake delivers `source_code`, parse `%udf_debug_level` and apply it with `tracing::level_filters::LevelFilter::set_max_level(level)` — an in-place global mutation that needs no `reload` layer and no extra feature flag. Add per-VM identity fields (`pid`, `node_id`, `session_id`, optional `vm_id`) to the formatter. Add a `udf_log!` macro and a default `UdfContext::debug_level()` trait method so UDF code can emit level-filtered lines to stderr (the stream the DB redirects). Add `debug`-gated RSS and emit-buffer telemetry plus emit/flush spans in `rowset.rs`.

#### Architecture

```
DB engine (zmqinternal.cc)              exaudfclient process
  SET SESSION SCRIPT OUTPUT ADDRESS
   → open TCP socket
   → posix_spawn dup2(socket → fd 1, fd 2)   main(): tracing fmt → stderr (fd 2)
   → spawn nschroot → exaudfclient ─────────▶   │  + pid/node_id/session_id/vm_id fields
                                                 ▼
                                            Runtime::run(): handshake → source_code
                                                 │  parse %udf_debug_level
                                                 │  LevelFilter::set_max_level(level)
                                                 ▼
                                            run loop / emit path (rowset.rs)
                                                 │  debug-gated RSS + emit-buffer telemetry
                                                 │  emit/flush spans
                                            UDF code: udf_log!(ctx, debug, ...) → stderr
                                                 │  gated by ctx.debug_level()
   listener receives everything on fd 2 ◀────────┘  (incl. startup errors, hard aborts)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Directive parsing from `source_code` | `exa-udf-runtime/src/artifact.rs` | Reuses the established `%udf_object` channel; no UDF code or rebuild |
| `LevelFilter::set_max_level` | `exa-udf-runtime/src/lib.rs` post-handshake | One-line global verbosity change; no reload layer, no Box<dyn Layer>, no feature flag |
| stderr-only sink, DB owns transport | `exaudfclient/src/main.rs` | The DB's fd-level redirect captures stderr; the SLC writes once and is done |
| Default trait method for `debug_level` | `exasol-udf-sdk/src/context.rs` | Existing UDFs compile unchanged; vtable change covered by routine version bump |
| Host-side telemetry/spans | `exa-udf-runtime/src/rowset.rs` | No `.so` boundary crossing; the emit path runs host-side |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| DB fd-level redirect provides the stream | SLC-managed `%udf_debug_output` + reload TCP layer | The DB `dup2`s the socket before spawn, capturing startup errors and hard aborts an SLC layer cannot. An SLC sink reimplements a working DB feature. |
| No crash-report subsystem | Two-mechanism crash reporting (alloc/signal handlers, BucketFS PUT) | The fd-2 redirect already delivers panic/abort/signal stderr to the listener; a bespoke subsystem is redundant complexity. |
| `LevelFilter::set_max_level` for runtime level | `tracing_subscriber::reload` layer swap | `set_max_level` is one line, no extra feature, no handle threaded into `Runtime`. The level is the only thing that changes post-handshake. |
| `%udf_debug_level` directive | `RUST_LOG` env var / `set_var` before init | The author controls `CREATE SCRIPT`, not the cluster process env; `RUST_LOG` is read before the handshake delivers the source. |
| `udf_log!` writes to stderr | Share the host `tracing` dispatcher across `dlopen` | Cross-`.so` dispatcher sharing is the banned fragile static-identity pattern; stderr lands directly in the DB-redirected stream. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| debug-output | NEW | `runtime/debug-output/spec.md` |

## Dependencies

- No new crate dependencies. `tracing` already provides `LevelFilter::set_max_level`; `tracing-subscriber` keeps its current `["env-filter"]` features (the `reload` feature is **not** added).
- Adding `UdfContext::debug_level()` changes the `dyn UdfContext` vtable. Per project rules every change bumps `[workspace.package].version` and the pinned `exasol-udf-sdk` entry; the ABI fingerprint is `SDK_VERSION:RUSTC_HASH`, so the version bump regenerates it — no separate fingerprint work. `debug_level()` has a default body, so existing UDFs compile unchanged.

## Implementation Tasks

1. **Parse the debug-level directive** in `crates/exa-udf-runtime/src/artifact.rs`:
   - [ ] 1.1 Add `parse_debug_level(source: &str) -> tracing::Level` (first `%udf_debug_level <level>`; maps `debug|info|warn|error`; trims trailing `;`; defaults to `info` when absent or unrecognised), mirroring `parse_udf_object_path`.
   - [ ] 1.2 Add unit tests: present / absent / trailing-semicolon / unrecognised-level cases.
   - [ ] 1.3 Re-export `parse_debug_level` from `crates/exa-udf-runtime/src/lib.rs`.

2. **Apply the level post-handshake** in `crates/exa-udf-runtime/src/lib.rs`:
   - [ ] 2.1 After `handshake` returns `meta`, call `parse_debug_level(&meta.source_code)` and `tracing::level_filters::LevelFilter::set_max_level(level)`.
   - [ ] 2.2 Confirm the `main()` subscriber is unchanged (`fmt().with_writer(std::io::stderr).with_env_filter(...).init()`); no `reload` wrapper, no `Runtime` handle field.

3. **Per-VM line tagging** in `crates/exaudfclient/src/main.rs` (formatter) and `crates/exa-zmq-protocol/src/meta.rs`:
   - [ ] 3.1 Configure the stderr `fmt` layer to include `pid` (`std::process::id()`), `node_id`, and `session_id` as fields on every line; include `vm_id` when available. A process-wide root span carrying these fields is acceptable.
   - [ ] 3.2 Parse `vm_id` into `UdfMeta` from the proto `ExascriptInfo` field 9 in `UdfMeta::from_pb`; if declined, document and use the `pid + node_id + session_id` fallback. Add public accessors for `session_id`, `node_id`, `vm_id` (currently `pub(crate)`).
   - [ ] 3.3 Confirm R6: the stderr writer flushes per write (no userspace `BufWriter`); `with_writer(std::io::stderr)` already does. Document the guarantee inline / in docs.

4. **SDK logging surface** in `crates/exasol-udf-sdk/`:
   - [ ] 4.1 Add a default `fn debug_level(&self) -> tracing::Level { tracing::Level::INFO }` to `UdfContext` in `src/context.rs`.
   - [ ] 4.2 Add a `udf_log!(ctx, debug|info|warn|error, ...)` macro in `src/lib.rs` (or `src/macros.rs`) that writes the formatted message to stderr only when `ctx.debug_level()` permits the level.
   - [ ] 4.3 Implement `debug_level()` on `HostContextBridge` (and `SingleCallContext`) in `crates/exa-udf-runtime/src/rowset.rs` to return the session's resolved level (read from `LevelFilter::current()` or a stored field). [expert]
   - [ ] 4.4 Bump `[workspace.package].version` and the pinned `exasol-udf-sdk` `[workspace.dependencies]` entry; regenerate `Cargo.lock`. Confirm `EXA_SDK_FINGERPRINT` reflects the new version and the loader's ABI check still rejects a stale UDF with a clear error; update any `validate.rs`/`loader.rs` test pinned to the old version.

5. **Memory + emit-buffer telemetry (R3) and emit/flush spans (R4)** in `crates/exa-udf-runtime/src/rowset.rs`:
   - [ ] 5.1 Read process RSS from `/proc/self/statm`; expose `EmitBuffer.byte_estimate`, cumulative emitted bytes, and row/batch counts to the telemetry path.
   - [ ] 5.2 Emit a `debug`-level event reporting RSS + emit-buffer estimate + cumulative bytes + row/batch counts at emit-path phase transitions and a periodic checkpoint; suppressed when the resolved level is above `debug`.
   - [ ] 5.3 Add tracing spans/events around `EmitBuffer::push` / `push_batch` and the `MT_EMIT` flush recording bytes buffered, bytes flushed, and flush outcome; host-side only.
   - [ ] 5.4 Integration test: at `debug` the telemetry appears, at `info` it does not.

6. **Documentation** in `docs/`:
   - [ ] 6.1 Add a debugging section explaining `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` as *the* output-redirect mechanism — a DB session attribute that redirects fd 1/fd 2 before spawn, so the Rust SLC benefits automatically and captures even startup crashes. Worked example: `nc -l 5000` → `SET SESSION SCRIPT OUTPUT ADDRESS 'mydev.local:5000'` → run UDF → observe all output.
   - [ ] 6.2 Document `%udf_debug_level` and the `udf_log!` / `ctx.debug_level()` surface. Contrast with the Python SLC: Python has the script-body `exa.redirect_output()`; Rust has no script body, but the DB-level mechanism captures more (including binary crashes).

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 (parse `%udf_debug_level` + tests), Task 3.2 (vm_id + accessors) |
| Group B | Task 2 (apply level), Task 3.1/3.3 (formatter tagging + flush) |
| Group C | Task 4 (SDK `udf_log!` + `debug_level` + version bump) |
| Group D | Task 5 (R3/R4 telemetry + spans) |
| Group E | Task 6 (docs) |

Sequential dependencies:
- Group A → Group B (applying the level and tagging use the parser and the `vm_id`/accessors)
- Group A → Group C (the host `debug_level()` impl returns the resolved level set in Group B; Group C's macro depends on the trait method)
- Group B → Group D (telemetry is gated on the resolved level applied in Group B)
- Groups A–D → Group E (docs describe the finished surface)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Spec | `specs/_plans/add-debug-output-redirect/runtime/crash-report/spec.md` | Deleted — the DB's fd-2 redirect captures panic/abort/signal output; the crash-report subsystem it described is unnecessary and misleading. |

No production code is removed; the change is additive. The `main()` subscriber is kept as-is (no reload wrapper was ever merged).

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| The database redirect captures all UDF process output | Integration | `crates/it/tests/debug_output.rs` | `db_output_address_captures_udf_stderr` |
| Debug level directive sets the runtime verbosity | Unit | `crates/exa-udf-runtime/src/artifact.rs` | `parses_debug_level_with_default` |
| The resolved level changes the global max verbosity at runtime | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `resolved_level_sets_global_max_level` |
| UDF code log calls reach the stderr stream | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `udf_log_macro_writes_to_stderr_when_permitted` |
| The context exposes the resolved debug level to UDF code | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `context_reports_resolved_debug_level` |
| Every runtime line is tagged with its origin VM | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `runtime_lines_carry_vm_tags` |
| Runtime lines are flushed individually | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `runtime_lines_flushed_per_write` |
| Memory and emit-buffer telemetry is emitted at debug level | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `telemetry_emitted_at_debug_level_only` |
| The emit and flush path is instrumented | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `emit_flush_path_instrumented` |

- The unit scenario (`parse_debug_level`) is pure string parsing with no I/O.
- The runtime-level scenarios use an in-process tracing capture writer (a `Mutex<Vec<u8>>` sink) and `LevelFilter` assertions — host-level, no Docker DB.
- "The database redirect captures all UDF process output" requires a live Exasol Docker container (it exercises a real `SET SESSION SCRIPT OUTPUT ADDRESS` redirect against a registered UDF), so it lives in `crates/it` under the `integration` feature and **fails** (not skips) if the DB is unavailable.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| debug-output | On dev host: `nc -l 5000`; in the SQL session run `SET SESSION SCRIPT OUTPUT ADDRESS 'mydev.local:5000'`, then run any Rust UDF query | Runtime tracing (handshake, resolved udf object, run-loop lines) appears live in the `nc` terminal during the query |
| debug-output | `nc -l 5000`; `SET SESSION SCRIPT OUTPUT ADDRESS 'mydev.local:5000'`; register a UDF whose `.so` fails to load (corrupt `%udf_object` path), run it | The startup/load error appears in the `nc` terminal even though no UDF code ran — proving the DB redirects fd 2 before spawn |
| debug-output | Register a script with `%udf_debug_level debug`, run an emit-heavy UDF with `SET SESSION SCRIPT OUTPUT ADDRESS` pointed at `nc` | RSS and emit-buffer byte-estimate telemetry lines appear and climb as rows are emitted; with `%udf_debug_level info` they do not |
| debug-output | Build a UDF that calls `udf_log!(ctx, debug, "checkpoint {}", n)`, register with `%udf_debug_level debug`, run with `SET SESSION SCRIPT OUTPUT ADDRESS` pointed at `nc` | The `checkpoint` lines, tagged with `pid`/`node_id`/`session_id`, appear in the `nc` terminal — proving UDF-side logs reach the redirect via stderr without a `.so`-local subscriber |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release -p exaudfclient` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures (fails if Docker DB unavailable) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
