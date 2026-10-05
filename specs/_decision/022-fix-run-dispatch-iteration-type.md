# Decisions: fix-run-dispatch-iteration-type

## ADR: Runtime iteration-axis gating instead of compile-time shape typing

**ID:** runtime-iteration-axis-gating
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

`UdfMeta::input_iter`/`output_iter` resolve from the handshake at run time. The dispatcher must still reject `next()` in scalar input and `emit()` in RETURNS output.

### Decision

Dispatch branches on `UdfMeta::input_iter`/`output_iter` at run time, and the host bridge enforces both context contracts. `UdfContext` and `UdfRun` have no shape-specific traits or generics.

### Options Considered

| Option | Verdict |
|--------|---------|
| Runtime iteration-axis gating | ✓ Chosen: matches the reference containers, SDK API unchanged |
| Compile-time shape typing (separate scalar/set traits or a typed `UdfRun`) | ✗ Shape is known only at run time; churns the author-facing SDK |

### Consequences

A shape mismatch surfaces as an `F-UDF-CL-RUST-` error at run time, not a compile error.

## ADR: RETURNS output uses a real value-return channel; emit() is banned in RETURNS

**ID:** returns-value-channel-emit-banned
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

The reference containers (Python, Lua, Java) have `run()` return a value in RETURNS context and reject `emit()` there. Routing RETURNS output through `ctx.emit()` leaves a two-row result possible and checked only at run time.

### Decision

A RETURNS function returns `Result<Option<T>, UdfError>`: `None` is SQL NULL, `Some(v)` is the single output row. The framework delivers the value via `UdfContext::set_return`, and `ctx.emit()` in RETURNS context returns `Err(UdfError)`. EMITS functions return `Result<(), UdfError>` and output via `ctx.emit()`. The macro-generated shim records the output shape (RETURNS or EMITS) in the vtable, and the loader validates it against `meta.output_iter`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Value-return channel, `emit()` banned in RETURNS | ✓ Chosen: matches the reference containers; two-row RETURNS is impossible by type |
| Emit-count contract (0 is NULL, 1 is value, 2 or more is error) | ✗ Keeps `emit()` as the RETURNS path; diverges from the reference |
| Interpreter-level emit ban | ✗ Unavailable in a compiled Rust SDK |

### Consequences

Enforcement uses Rust types plus a load/run shape check, not an interpreter ban. A shape mismatch is a clear error.

## ADR: Group boundary anchored to the MT_RUN/MT_DONE outer loop

**ID:** group-boundary-mt-run-mt-done
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

Set dispatch must know where an input group ends so `ctx.next()` spans batches within a group and stops at its boundary.

### Decision

Each `MT_RUN`-opened iteration is one input group, and the `MT_DONE` answering `MT_NEXT` is that group's input exhaustion, per `docs/protocol.md`. `ctx.next()` and the scalar per-row loop span the group's `MT_NEXT` batches and stop there. `rows_in_group` is not the group-boundary mechanism.

### Options Considered

| Option | Verdict |
|--------|---------|
| `MT_RUN`/`MT_DONE` outer loop | ✓ Chosen: matches `docs/protocol.md` |
| Track `rows_in_group` to delimit groups within one `MT_RUN` | ✗ The live-DB multi-group GROUP BY conformance test is the oracle, not the wire mechanism |

### Consequences

Set aggregation yields one aggregate per GROUP BY group, not one per `MT_NEXT` batch.

## ADR: Emit buffer scoped to the input group, flushed before each group's MT_DONE

**ID:** emit-buffer-scoped-to-input-group
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

Scalar dispatch calls `run()` once per row, so a flush per `run()` sends one `MT_EMIT` per row. A set group's output must not leak into a later group.

### Decision

The `EmitBuffer` is scoped to the whole input group, accumulating across scalar per-row invocations and a set group's batches. It flushes at a `4_000_000` byte threshold and once more before the group's `MT_DONE`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Buffer scoped to the input group | ✓ Chosen |
| Flush after every `run()` | ✗ One `MT_EMIT` per scalar row defeats buffering |
| Buffer across all groups | ✗ The database misattributes output to a later group |

### Consequences

The dispatcher tracks a group-scoped buffer lifecycle, not a per-`run()` one.
