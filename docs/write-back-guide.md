[language-container-rs](../README.md) · [Docs](index.md)

# Write-back from a UDF (connect-back)

"Write-back" = a UDF modifying the database (INSERT/UPDATE/DDL) from inside
`run()` by opening a **connect-back** session. This guide covers how to do it
correctly; the mechanics of the three connect-back methods are in
[Writing a Rust UDF §4](writing-a-udf.md#4-connect-back).

## The one rule that explains everything

**Connect-back is a brand-new, ordinary SQL login** — exactly what PyExasol,
JDBC or any external client does. It runs in its **own session and its own
transaction**, completely independent of the query that invoked the UDF. There
is no special "talk to my parent transaction" hook; Exasol does not support one.

Everything below follows from that fact plus Exasol's **Serializable** isolation
level.

## Serializable isolation — the constraints

Exasol serializes transactions. Each runs as if part of a sequence even when
parallel. Two consequences bite write-back UDFs:

- A transaction may have to **WAIT FOR COMMIT** of an earlier one before it can
  proceed.
- Mixed read/write transactions can **collide**, forcing a rollback.

Because the connect-back session is a *separate* transaction from the invoking
query, you must keep them from conflicting. If they conflict, the invoking
query's worker waits, then the deadlock detector aborts it — observed as the
SQL worker dying ~10 s after connect-back with a `deadlock detector signalled`
entry in the DB log.

### Rules for safe write-back

1. **Pre-create and commit the target table *before* the invoking query runs.**
   The connect-back session can only see committed objects. Doing `CREATE TABLE`
   inside the connect-back session, in the invoking query's schema, is DDL that
   conflicts with the query's schema locks → WAIT FOR COMMIT → abort.
2. **Do no DDL in the connect-back session.** Only DML against pre-existing,
   committed objects.
3. **Write a *different* object than the invoking query reads.** If the query is
   `SELECT my_udf(v) FROM input_t` and the UDF writes `result_t`, there is no
   read-write conflict. Writing the same table the query reads will collide.
4. **Same schema is fine** under rules 1–3. The target does *not* need a separate
   schema — it just needs to be pre-committed and distinct from what the query
   reads. (A separate schema is one easy way to guarantee rule 3, not a
   requirement.)
5. **Rely on autocommit; do not issue `COMMIT`.** exarrow-rs sessions autocommit
   by default, so each `execute()` commits on its own. An explicit
   `conn.execute("COMMIT")` errors — there is no open transaction — *after* the
   data has already landed, surfacing as a spurious UDF failure.

## Worked example

A `SET` UDF that squares each input and writes `(v, v*v)` into a pre-committed
table in its **own** schema (from `test-udfs/connect-back-crunch`):

```rust
#[exasol_udf]
pub fn crunch_writeback(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // Drain input first, then open the session.
    let mut vals = Vec::new();
    while ctx.next()? {
        vals.push(match ctx.get(0)? {
            Value::Int64(n) => *n,
            Value::Numeric(s) => s.parse().map_err(|e| UdfError::Type(format!("{e}")))?,
            _ => return Err(UdfError::Type("expected integer".into())),
        });
    }

    let c = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&c)?;

    let rows = vals.iter().map(|v| format!("({v}, {})", v * v))
        .collect::<Vec<_>>().join(", ");
    if !rows.is_empty() {
        // No COMMIT — autocommit handles it.
        conn.execute(&format!("INSERT INTO it_rust.crunch_log VALUES {rows}"))?;
    }

    // BIGINT EMITS columns travel as PB_NUMERIC — emit Numeric, not Int64.
    for _ in &vals { ctx.emit(&[Value::Numeric("1".to_string())])?; }
    Ok(())
}
```

Driver SQL that respects the ordering:

```sql
-- BEFORE: create + seed the target, committed (autocommit) before the query.
CREATE OR REPLACE TABLE it_rust.crunch_log (v BIGINT, v_squared BIGINT);
INSERT INTO it_rust.crunch_log VALUES (1, 1);

-- Separate input table the query reads (not crunch_log).
CREATE OR REPLACE TABLE it_rust.crunch_in (v BIGINT);
INSERT INTO it_rust.crunch_in VALUES (2), (3), (4);

-- RUN: connect-back inserts (2,4),(3,9),(4,16).
SELECT crunch_writeback(v) FROM it_rust.crunch_in;

-- AFTER: any new session can keep using the table.
INSERT INTO it_rust.crunch_log VALUES (5, 25);
```

## Reading data back

To read in a UDF, prefer the FFI-safe `query()` over `query_arrow()`:

```rust
let rows = conn.query("SELECT CAST(42 AS BIGINT)")?;   // Vec<Vec<Value>>
let n = match rows.first().and_then(|r| r.first()) {
    Some(Value::Numeric(s)) => s.parse::<i64>().unwrap_or(0),
    Some(Value::Int64(n))   => *n,
    _ => 0,
};
```

`query()` converts arrow → the SDK `Value` enum **inside the runtime** and
returns plain data. `query_arrow()` returns `arrow::RecordBatch` — see the
Pitfalls for why downcasting those in UDF code silently fails.

## Pitfalls / DX notes

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| DDL or write in the invoking query's schema, uncommitted before the query | SQL worker SIGABRT ~10 s after connect-back; `deadlock detector signalled` | Pre-create + commit target; no DDL in connect-back; write a table the query doesn't read |
| Explicit `COMMIT` on an autocommit session | UDF fails *after* data already committed | Remove the `COMMIT`; rely on autocommit |
| Emitting `Value::Int64` for a `BIGINT` column | DB SIGSEGV in `handle_emit_request` (reads empty string block) | Emit `Value::Numeric` — BIGINT is `PB_NUMERIC` on the wire |
| `query_arrow()` + `downcast_ref` in UDF code | Silent wrong value (e.g. `0` instead of `42`) | Use `query()` (returns `Value`s). The UDF `.so` links its own `arrow`; arrow `TypeId`s differ across the cdylib boundary, so `downcast_ref` returns `None` |
| Bare `error code 1` from a failed UDF | No detail on what failed | Known limitation — see [`specs/backlog.md`](../specs/backlog.md). Until fixed, instrument the UDF (sentinel values / a file write) or read `/exa/logs/db/DB1/*SqlSession*` |

See [`specs/backlog.md`](../specs/backlog.md) for tracked DX improvements
(notably propagating UDF error messages through the protocol).
