# Backlog

> Deferred work and known limitations that outlive any single plan. Unlike a
> plan's verification report (which is archived into `specs/_recorded/` by
> `/speq:record`), this file is permanent and sits beside `mission.md` and
> `decision-log.md`. Add an entry when you defer work; remove it when the work
> lands (ideally referencing the plan that closed it).

---

## UX / DX

### B-002: Prefer `query()` over `query_arrow()` in the connect-back API

**Raised by:** `fix-connect-back-version-matrix` (2026-06-10)
**Severity:** medium

`ExaConnection::query_arrow()` returns `arrow::RecordBatch`. Because a UDF `.so`
links its own copy of `arrow`, `downcast_ref::<…Array>()` on a runtime-produced
array silently returns `None` (different `TypeId`s across the cdylib boundary),
yielding wrong values with no error. The FFI-safe `query()` / `query_for_each()`
(return SDK `Value`s) now exist and are documented as the preferred API.
Consider deprecating `query_arrow()` for UDF use, or hiding it behind a
clearly-named "same-process only" gate.

### B-003: Richer connect-back errors with the failing SQL

**Raised by:** `fix-connect-back-version-matrix` (2026-06-10)
**Severity:** low

Connect-back `execute()` / `query()` errors surface the exarrow-rs error but not
the SQL statement that failed. The runtime has the statement in hand
([`crates/exa-udf-runtime/src/connect_back.rs`](../crates/exa-udf-runtime/src/connect_back.rs))
— wrap errors as `"<sql> failed: <err>"` to speed diagnosis.

---

## Test coverage

### B-004: Run the connect-back suite across the full version matrix locally

**Raised by:** `fix-connect-back-version-matrix` (2026-06-10)
**Severity:** low

`db_roundtrip_all_scenarios` is verified locally on `2026.1.0`. CI runs the
`2025.1.11` / `2025.2.1` / `2026.1.0` matrix; a local sweep across all three
(via `EXASOL_VERSION` / `EXASOL_DB_SERIES`) would catch version drift earlier.

---

## Performance

### B-005: Raw per-column buffers for `emit_batch` instead of Arrow IPC

**Raised by:** `add-emit-batch-arrow` (2026-06-24)
**Severity:** low

`emit_batch` serialises the UDF's `RecordBatch` to Arrow IPC bytes inside the
`.so` and the host deserialises into its own `RecordBatch` before `push_batch`
encodes it — because an Arrow `RecordBatch` cannot cross the cdylib boundary
safely (same root cause as [B-002]; crossing it SIGSEGVs/yields garbage as two
static `arrow` copies disagree on `Arc<dyn Array>` vtables and `TypeId`). IPC
costs two bulk columnar copies (serialise + deserialise) but eliminates the
per-cell downcast / per-`Value` alloc / per-row boundary crossing of row-based
`emit`, so it already beats the row path.

A faster-still variant skips the FlatBuffer framing **and** the host-side Arrow
rebuild: the UDF passes raw per-column buffers (value buffer + validity bitmap +
offsets for variable-width, with a small type tag) as `&[u8]` slices, and the
host reads those bytes straight into the proto type blocks — one fewer copy and
no `arrow-ipc` dependency on the hot path. Cost: we hand-roll and own a columnar
wire format per supported type instead of leaning on `arrow-ipc`. Only worth it
if profiling shows the IPC serialise/deserialise dominates for real batch sizes.
