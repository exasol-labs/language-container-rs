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
