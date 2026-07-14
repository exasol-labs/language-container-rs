[language-container-rs](../README.md) › [docs](index.md) › Debugging Rust UDFs

---

# Debugging Rust UDFs

## Output redirect: `SET SESSION SCRIPT OUTPUT ADDRESS`

This is the primary mechanism for capturing live UDF output. It is a **database feature**, not an SLC feature: the engine reads the session attribute, opens a TCP connection to your listener, and `dup2`s the socket onto the child process's fd 1 (stdout) and fd 2 (stderr) *before* `exaudfclient` is spawned. Because the redirect is wired before the process starts, it captures:

- Runtime `tracing` lines (handshake, run loop, directive parsing)
- Startup errors and ABI mismatches (before any UDF code runs)
- Hard crashes — panics, aborts, signal-time output — that no in-process handler could capture

The Rust SLC writes only to stderr. No SLC-side TCP code, no reload layer, no crash-report subsystem — writing to stderr is sufficient.

### Worked example

On your development host, start a listener on any free port:

```bash
nc -l 5000
```

In your SQL session, set the redirect and run a Rust UDF:

```sql
SET SESSION SCRIPT OUTPUT ADDRESS 'mydev.local:5000';

SELECT my_schema.scalar_double(21);
```

All runtime output for that query appears live in the `nc` terminal.

To verify the redirect captures pre-Rust output (e.g. a bad `%udf_object` path), register a script that points at a nonexistent `.so` and run it — the load error appears in `nc` even though no UDF code executed.

> The redirect is session-scoped. Clear it with `SET SESSION SCRIPT OUTPUT ADDRESS ''` or by closing the session.

---

## Verbosity: `%udf_debug_level`

Add a `%udf_debug_level` directive to the script source body to set the runtime tracing level for that UDF. It is parsed after the handshake, so it applies for the entire run without rebuilding the `.so`.

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.scalar_double(val BIGINT)
RETURNS BIGINT AS
%udf_debug_level debug
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

Accepted values: `debug`, `info`, `warn`, `error`. Default when absent or unrecognised: `info`.

A trailing semicolon is accepted: `%udf_debug_level debug;`.

At `debug` level the runtime additionally emits:
- RSS and emit-buffer byte estimates at emit-path phase transitions
- Per-push and per-flush spans with bytes buffered and flushed

---

## UDF-side logging: `udf_log!` and `ctx.debug_level()`

Use the `udf_log!` macro to write level-filtered lines to stderr from inside your UDF. The output lands in the same stream the DB redirect captures.

```toml
[dependencies]
exasol-udf-sdk    = { version = "0.19" }
exasol-udf-macros = { version = "0.19" }
```

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::udf_log;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    udf_log!(ctx, debug, "input col 0 = {:?}", ctx.get(0)?);
    let n = ctx.get_i64(0)?.unwrap_or(0);
    udf_log!(ctx, info, "doubling {}", n);
    Ok(Some(Value::Numeric(Decimal::from(n * 2))))
}
```

`udf_log!(ctx, level, fmt, args...)` writes a formatted line to stderr when `ctx.debug_level()` permits the given level. No subscriber is installed in the `.so`; the line goes directly to stderr.

`ctx.debug_level()` returns the resolved `tracing::Level` for the current UDF invocation (set by the `%udf_debug_level` directive, or `INFO` by default). You can use it to gate expensive per-row work:

```rust
if ctx.debug_level() >= tracing::Level::DEBUG {
    let row_repr = expensive_format(ctx)?;
    udf_log!(ctx, debug, "row = {}", row_repr);
}
```

Line format: every line is tagged with `pid`, `node_id`, and `session_id` (and `vm_id` when available) so you can de-interleave output from parallel VMs on a multi-node cluster.

---

## Contrast with the Python3 SLC

The Python3 SLC exposes `exa.redirect_output(host, port)` from the script body — the UDF code itself opens a TCP connection. The Rust SLC has no script body, so that approach is not available. Instead:

| | Python3 SLC | Rust SLC |
|---|---|---|
| Redirect mechanism | `exa.redirect_output(host, port)` in script body | `SET SESSION SCRIPT OUTPUT ADDRESS` DB session attribute |
| Captures startup crashes | No — connection opened after script starts | Yes — DB `dup2`s before spawn |
| Captures ABI/load errors | No | Yes |
| Verbosity control | `exa.set_output_level(...)` or similar | `%udf_debug_level` directive in `CREATE SCRIPT` |

The DB-level redirect is strictly broader: it captures everything the process writes to fd 2 regardless of when or how the process exits.
