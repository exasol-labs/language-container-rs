# Decisions: fix-run-dispatch-iteration-type

## ADR: Runtime iteration-axis gating instead of compile-time shape typing

**ID:** runtime-iteration-axis-gating
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

`UdfMeta::input_iter`/`output_iter` resolve from the handshake at run time, not at Rust compile time. The dispatcher must enforce scalar-versus-set and RETURNS-versus-EMITS context contracts (rejecting `next()` in scalar input, `emit()` in RETURNS output) without knowing the shape until a UDF loads.

### Decision

Branch dispatch on `UdfMeta::input_iter`/`output_iter` at run time and enforce both context contracts in the host bridge. Introduce no shape-specific traits or generics on `UdfContext`/`UdfRun`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Runtime iteration-axis gating | ✓ Chosen — matches the reference containers and keeps the SDK API unchanged |
| Compile-time shape typing (separate scalar/set context traits or a typed `UdfRun`) | ✗ Rejected — shape is only known from the handshake at run time; typing would churn the entire author-facing SDK for no correctness gain |

### Consequences

Enforcement stays localized to the runtime, so the SDK surface is stable and existing UDF crates keep compiling. The cost is that a shape mismatch surfaces at run time (an `F-UDF-CL-RUST-` error) rather than as a compile error.

## ADR: RETURNS output uses a real value-return channel; emit() is banned in RETURNS

**ID:** returns-value-channel-emit-banned
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

The prior emit-count contract (0 emits → NULL, 1 → value, ≥2 → error) kept `ctx.emit()` as the RETURNS output path. Every reference container (Python, Lua, Java) instead has `run()` return a value and rejects `emit()` in scalar/RETURNS context, so a two-row RETURNS bug was only a runtime count check rather than a structural impossibility.

### Decision

A RETURNS function returns `Result<Option<T>, UdfError>`: `None` maps to SQL NULL, `Some(v)` to the single output row. The framework delivers that value via `UdfContext::set_return`, and any author call to `ctx.emit()` in RETURNS context returns `Err(UdfError)`. EMITS functions are unchanged (`Result<(), UdfError>`, output via `ctx.emit()`).

### Options Considered

| Option | Verdict |
|--------|---------|
| Value-return channel with author `emit()` banned in RETURNS | ✓ Chosen — matches every reference container; a two-row RETURNS becomes a type impossibility, not a count check |
| Emit-count contract (0→NULL, 1→value, ≥2→error) | ✗ Rejected — kept `emit()` as the RETURNS path, diverging from the reference semantics |
| Interpreter-level emit ban (as the reference implements it) | ✗ Rejected — not available in a compiled Rust SDK |

### Consequences

RETURNS UDFs read as idiomatic Rust (the function returns its value), `None → NULL` is first-class, and the compiled output shape is validated against `meta.output_iter` at load/run, turning a mismatch into a clear error instead of undefined behavior. Enforcement relies on Rust types plus a load/run check rather than an interpreter-level ban, but the observable semantics match the reference.

## ADR: Group boundary anchored to the MT_RUN/MT_DONE outer loop

**ID:** group-boundary-mt-run-mt-done
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

Set dispatch must know where one input group ends and the next begins so `ctx.next()` spans batches within a group but stops at the group boundary. The wire carries a `rows_in_group` proto field as an alternative signal to delimit groups within a single `MT_RUN`.

### Decision

Treat each `MT_RUN`-opened iteration as one input group and the `MT_DONE` that answers `MT_NEXT` as that group's input exhaustion, per `docs/protocol.md`. `ctx.next()` and the scalar per-row loop span the group's `MT_NEXT` batches and stop at that boundary. `rows_in_group` remains an implementation detail, not the group-boundary mechanism.

### Options Considered

| Option | Verdict |
|--------|---------|
| `MT_RUN`/`MT_DONE` outer loop as the group boundary | ✓ Chosen — matches the repo's own protocol documentation |
| Track `rows_in_group` to delimit multiple groups within a single `MT_RUN` | ✗ Rejected — the live-DB multi-group GROUP BY conformance test is the oracle; the spec fixes observable behavior, not the wire mechanism |

### Consequences

Set aggregation across GROUP BY groups produces one correct aggregate per group instead of one partial result per `MT_NEXT` batch. The live-DB conformance suite, not a documentation reading, is the source of truth for whichever wire mechanism an implementer adopts.

## ADR: Emit buffer scoped to the input group, flushed before each group's MT_DONE

**ID:** emit-buffer-scoped-to-input-group
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

Per-row scalar `run()` invocations must not each trigger a separate `MT_EMIT` frame, and a set group's output must not leak into a later group. The CLAUDE.md rule "flush at end of `run()`" no longer maps cleanly to a single `run()` call once scalar dispatch invokes `run()` once per row.

### Decision

Scope the `EmitBuffer` to the whole input group, accumulating across scalar per-row invocations and across a set group's batches. Threshold-flush at `4_000_000` bytes and tail-flush once before the group's `MT_DONE`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Emit buffer scoped to the input group | ✓ Chosen — reconciles "flush at end of run" (reread as "end of the group's iteration") with per-row scalar invocation |
| Flush after every `run()` | ✗ Rejected — sends one `MT_EMIT` per scalar row, defeating buffering |
| Buffer across all groups | ✗ Rejected — lets the DB misattribute one group's output to a later group |

### Consequences

Output stays correctly attributed to its group and batched efficiently regardless of dispatch shape. The dispatcher must track a group-scoped buffer lifecycle instead of a per-`run()`-call one.

## ADR: Return value crosses the .so boundary via a dedicated context method, not the emit path

**ID:** return-value-set-return-not-emit
**Plan:** fix-run-dispatch-iteration-type
**Status:** Accepted

### Context

The RETURNS value-return channel needs a way to deliver the UDF's returned value to the host bridge across the `.so` boundary, distinct from the author-facing `emit()` the bridge must reject in RETURNS context. An arbitrary `Value` (Numeric, Timestamp, String) has no simple C-ABI blob form.

### Decision

The macro-generated RETURNS shim converts the returned `Option<T>` to `Option<Value>` and delivers it through a dedicated `UdfContext::set_return(&mut self, value: Option<Value>)` method (default `Unimplemented`), separate from `emit()`. A new `ExaUdfVTable` output-shape marker records RETURNS versus EMITS, validated against `meta.output_iter`. `EXA_UDF_ABI_VERSION` bumps `6 → 7`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Dedicated `set_return` method on `UdfContext` | ✓ Chosen — keeps the author-`emit()` ban cleanly separable and reuses the proven `Value`-over-trait-object path |
| Reuse `ctx.emit()` internally for the returned row | ✗ Rejected — the bridge could not distinguish the framework's sanctioned emit from a banned author `emit()` |
| Return-value out-pointer on the `run` vtable signature | ✗ Rejected — an arbitrary `Value` has no simple C-ABI blob form, unlike the existing trait-object vtable path |

### Consequences

The ban on author `emit()` in RETURNS stays structurally enforceable, and a compiled/registered output-shape mismatch fails loudly at load/run rather than corrupting the wire. The ABI version bump means a `.so` built against ABI 6 must be rebuilt before it loads under this host.
