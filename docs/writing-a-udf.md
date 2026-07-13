[language-container-rs](../README.md) › [docs](index.md) › Writing a Rust UDF

---

# Writing a Rust UDF

## Prerequisites

- Rust 1.94+ with the musl target:
  ```bash
  rustup target add x86_64-unknown-linux-musl
  ```
- `cargo-exasol-udf` installed from crates.io (or `--path crates/cargo-exasol-udf` from this workspace):
  ```bash
  cargo install cargo-exasol-udf
  ```
- A running Exasol cluster with BucketFS write access.

## 1. Scaffold a UDF crate

```bash
cargo exasol-udf new my-udf
cd my-udf
```

Or create the crate manually. The crate must be a `cdylib`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
exasol-udf-sdk    = { version = "0.11" }
exasol-udf-macros = { version = "0.11" }
```

## 2. The `#[exasol_udf]` macro

Annotate a public function with `#[exasol_udf]`. The macro generates the C ABI entry point and vtable the runtime expects, and reads the function's return type to pick the output shape (see §6): `Result<Option<T>, UdfError>` selects RETURNS; `Result<(), UdfError>` selects EMITS.

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn my_udf(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    // ...
    Ok(None)
}
```

The function name becomes the SQL script name (case-insensitive). `Ok(Some(v))` sets the RETURNS output value; `Ok(None)` maps to SQL `NULL`.

### Multiple UDFs per `.so`

A single cdylib can export any number of `#[exasol_udf]`-annotated functions. Each function generates its own C entry point and vtable; they never conflict:

```rust
#[exasol_udf(input(x: i64))]
pub fn identity(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> { /* … */ }

#[exasol_udf(input(x: i64))]
pub fn double_it(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> { /* … */ }
```

Both entry points live in the same `.so`. Each is registered as a separate SQL script:

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.identity(x BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/

CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.double_it(x BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

### Entry-point naming and the `name` attribute

The macro derives the exported C symbol from the Rust function name: it converts the identifier to `UPPER_SNAKE_CASE` and prefixes it with `__exa_udf_entry_`. For example, `fn double_it` → `__exa_udf_entry_DOUBLE_IT`.

Use `name = "..."` to override the derived SQL name (and the symbol suffix) without renaming the Rust function:

```rust
#[exasol_udf(name = "MY_SPECIAL_UDF", input(x: i64))]
pub fn internal_impl(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> { /* … */ }
// exports __exa_udf_entry_MY_SPECIAL_UDF
```

The SQL `CREATE SCRIPT` name must match exactly (case-insensitive on the SQL side, but the symbol suffix is verbatim):

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.my_special_udf(x BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

> **Upgrade note (sdk < 0.14.0):** Versions before 0.14.0 exported a single bare `__exa_udf_entry` symbol. If you have an existing `.so` built against an older SDK, rebuild it with sdk >= 0.14.0 before registering it with a `CREATE SCRIPT` that uses the named entry-point convention. The runtime will reject a `.so` that lacks the expected `__exa_udf_entry_<NAME>` symbol with a clear error message.

### Optional type annotations

If you annotate the input types, the runtime validates the SQL input-column schema at load time:

```rust
#[exasol_udf(input(val: i64))]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    // ...
}
```

For an EMITS UDF, annotate the output columns too:

```rust
#[exasol_udf(input(k: i64), emits(i: i64))]
pub fn emit_k(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // ...
}
```

Supported annotation types: `i32`, `i64`, `f64`, `f32`, `bool`, `String`, `&str`, `Decimal`, `NaiveDate`, `NaiveDateTime`.

## 3. The `UdfContext` interface

Every UDF receives `&mut dyn UdfContext`. The four core operations are:

| Method | What it does |
|--------|-------------|
| `ctx.get(col)` | Returns `&Value` for column `col` (0-indexed) on the current input row |
| `ctx.emit(values)` | Appends one output row. Valid only for EMITS output — the host returns `Err` if a RETURNS UDF calls it (see §6) |
| `ctx.next()` | Advances to the next input row of a SET group, spanning `MT_NEXT` batches; returns `false` at the group boundary. Valid only for SET input — the host returns `Err` if a SCALAR UDF calls it |
| `ctx.set_return(value)` | Sets the invocation's single RETURNS output value. The `#[exasol_udf]` macro calls this for you from the function's `Ok(Some(v))` / `Ok(None)` return; you never call it directly |

`next()` is for SET UDFs only — call it before the first `get()` on each row; it walks every row of the group across all `MT_NEXT` batches transparently. SCALAR UDFs start with the single input row already loaded and must not call `next()`. See §6 for the full RETURNS/EMITS × SCALAR/SET matrix.

### Typed getters

`ctx.get(col)` returns the raw `Value` enum. The typed getters unwrap it and return `Ok(None)` for SQL `NULL`:

```rust
fn get_i64(&self, col: usize)      -> Result<Option<i64>,           UdfError>;
fn get_f64(&self, col: usize)      -> Result<Option<f64>,           UdfError>;
fn get_str(&self, col: usize)      -> Result<Option<&str>,          UdfError>;
fn get_bool(&self, col: usize)     -> Result<Option<bool>,          UdfError>;
fn get_decimal(&self, col: usize)  -> Result<Option<Decimal>,       UdfError>;
fn get_date(&self, col: usize)     -> Result<Option<NaiveDate>,     UdfError>;
fn get_datetime(&self, col: usize) -> Result<Option<NaiveDateTime>, UdfError>;
```

Each returns `UdfError::Type` if the column holds a different variant. Exception: `get_i64` deliberately accepts a `Value::Numeric` with `scale == 0` — Exasol sends `BIGINT` as `PB_NUMERIC` on the wire, so this is the normal case (see the Value reference below).

## 4. Value enum reference

`Value` is the runtime representation of any SQL column value:

| Variant | Rust type | SQL type(s) |
|---------|-----------|-------------|
| `Value::Int32(i32)` | `i32` | `DECIMAL(p,0)` fitting i32 |
| `Value::Int64(i64)` | `i64` | `DECIMAL(p,0)` fitting i64 |
| `Value::Numeric(Decimal)` | `Decimal` | `DECIMAL(p,s)`, **`BIGINT`**, `NUMBER` |
| `Value::Double(f64)` | `f64` | `DOUBLE PRECISION`, `FLOAT`, `REAL` |
| `Value::Bool(bool)` | `bool` | `BOOLEAN` |
| `Value::String(String)` | `String` | `VARCHAR`, `CHAR`, `GEOMETRY`, `HASHTYPE`, interval types |
| `Value::Date(NaiveDate)` | `chrono::NaiveDate` | `DATE` |
| `Value::Timestamp(NaiveDateTime)` | `chrono::NaiveDateTime` | `TIMESTAMP` |
| `Value::Null` | — | SQL `NULL` |

> **BIGINT arrives as `Value::Numeric`**, not `Value::Int64`. Exasol sends BIGINT over the wire as `PB_NUMERIC`, a scale-0 decimal. Use `get_i64()` for BIGINT input columns — it accepts both `Int64` and a scale-0 `Numeric`. When emitting a BIGINT output column, use `Value::Numeric(Decimal::from(n))`, not `Value::Int64`.

## 5. SDK type reference

### `Decimal`

`Decimal` backs `Value::Numeric`. It is a `{ unscaled: i128, scale: u8 }` struct — 38-digit precision, no heap allocation. The value is `unscaled × 10⁻ˢᶜᵃˡᵉ`.

```rust
use exasol_udf_sdk::value::Decimal;

let d = Decimal::from_str("3.14")?;   // parse
let d = Decimal::from_f64(3.14)?;     // from float (lossy)
let d = Decimal::from(42_i64);        // scale-0 integer (BIGINT)

// arithmetic via fields
let doubled = Decimal { unscaled: d.unscaled * 2, scale: d.scale };

println!("{d}");   // "3.14"
```

### `ExaType`

`ExaType` is the column-level SQL type, independent of the wire value. It surfaces in typed `#[exasol_udf]` annotations and validation. The full variant set covers `Double`, `Int32`, `Int64`, `Numeric { precision, scale }`, `Boolean`, `String { size }`, `Char { size }`, `Date`, `Timestamp`, `TimestampTz`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, and `Unsupported`.

Most UDFs do not need `ExaType` — the `Value` variant and typed getters carry enough information.

### `UdfError`

```rust
pub enum UdfError {
    User(String),          // UDF logic error; use for domain errors
    Type(String),          // wrong Value variant for a getter
    Unimplemented(String), // SDK feature not available in this context
    ConnectBack(String),   // connect-back failure
}
```

Return `Err(UdfError::User("message".into()))` for expected errors. Use `?` to propagate errors from typed getters and connect-back calls.

## 6. The four dispatch shapes

A UDF's dispatch shape is a product of two independent axes: how many input rows `run()` sees per invocation (SCALAR vs SET), and how a result leaves `run()` (RETURNS vs EMITS). The `#[exasol_udf]` macro derives the output axis from the function's return type; the `CREATE SCRIPT` statement declares both axes via `SCALAR`/`SET` and `RETURNS <type>`/`EMITS (...)`.

| | RETURNS — `Result<Option<T>, UdfError>` | EMITS — `Result<(), UdfError>` |
|---|---|---|
| **SCALAR** — `run()` once per input row | §7 `scalar_double` | §9 `emit_k` |
| **SET** — `run()` once per input group; `ctx.next()` walks rows across batches | §8 `set_sum` | §10 `set_filter` |

`Ok(Some(v))` sets the RETURNS value for this invocation; `Ok(None)` maps to SQL `NULL`. Calling `ctx.emit` in RETURNS output returns `Err` — the return value is the only output channel. Calling `ctx.next()` in SCALAR input returns `Err` — the framework has already loaded the single row before `run()` starts.

## 7. Scalar RETURNS example

A SCALAR UDF's `run()` executes once per input row; the single row is already loaded when `run()` starts.

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    let doubled = match ctx.get(0)? {
        Value::Int64(n) => Value::Int64(n * 2),
        // Exasol sends BIGINT as PB_NUMERIC (typed Decimal with scale=0).
        Value::Numeric(d) if d.scale == 0 => {
            let n = i64::try_from(d.unscaled)
                .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))?;
            Value::Numeric(Decimal {
                unscaled: (n * 2) as i128,
                scale: 0,
            })
        }
        Value::Null => return Ok(None),
        _ => return Err(UdfError::Type("expected Int64 or Numeric".into())),
    };
    Ok(Some(doubled))
}
```

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.scalar_double(val BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/

SELECT my_schema.scalar_double(21);  -- 42
```

`Ok(Some(v))` becomes the row's output value; `Ok(None)` becomes SQL `NULL`. Calling `ctx.emit` here fails at runtime — RETURNS output has no `emit` channel. See `test-udfs/scalar-double` for the full fixture and its unit tests.

## 8. Set RETURNS example

A SET UDF's `run()` executes once per input group. Call `ctx.next()` to walk the group's rows — it spans every `MT_NEXT` batch in the group transparently, so the UDF never sees a batch boundary.

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

#[exasol_udf]
pub fn set_sum(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    let mut sum: i64 = 0;
    while ctx.next()? {
        if let Some(n) = ctx.get_i64(0)? {
            sum += n;
        }
    }
    Ok(Some(sum))
}
```

```sql
CREATE OR REPLACE RUST SET SCRIPT my_schema.set_sum(val BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/

SELECT my_schema.set_sum(v) FROM my_table GROUP BY grp;
```

One `set_sum` invocation runs per `grp` value and returns one aggregate row per group — the same `Some`/`None` value/NULL contract as §7's SCALAR RETURNS, just spanning a whole group instead of a single row. See `test-udfs/set-sum` for the full fixture.

**Cluster distribution** — Exasol executes SET UDFs on every node in parallel. Grouping by [`IPROC()`](https://docs.exasol.com/db/latest/sql_references/functions/alphabeticallistfunctions/iproc.htm) pins each group to the node that owns the data, saturating the full cluster with a single query.

## 9. Scalar EMITS example

A SCALAR UDF can still emit a variable number of output rows per input row — the input axis (one row per invocation) and the output axis (RETURNS vs EMITS) are independent.

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn emit_k(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let k = ctx.get_i64(0)?.unwrap_or(0);
    for i in 0..k {
        ctx.emit(&[Value::Int64(i)])?;
    }
    Ok(())
}
```

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.emit_k(k BIGINT)
EMITS (i BIGINT) AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/

SELECT my_schema.emit_k(k) EMITS (i BIGINT)
FROM (SELECT 0 AS k UNION ALL SELECT 1 UNION ALL SELECT 3) t;
-- k=0 -> 0 output rows
-- k=1 -> 1 row:  i=0
-- k=3 -> 3 rows: i=0, i=1, i=2
```

Each invocation emits `k` rows, so the same query produces 0, 1, or many output rows per input row. See `test-udfs/emit-k` for the full fixture.

## 10. Set EMITS example

A SET UDF that drives `ctx.next()` and calls `ctx.emit` for the rows it keeps — the row-based counterpart to Arrow-batch emission (§11).

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn set_filter(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {
        match ctx.get(0)? {
            Value::Int64(n) if *n > 0 => ctx.emit(&[Value::Int64(*n)])?,
            // Exasol sends BIGINT as PB_NUMERIC (typed Decimal with scale=0).
            Value::Numeric(d) if d.scale == 0 => {
                let n = i64::try_from(d.unscaled)
                    .map_err(|_| UdfError::Type(format!("cannot convert {} to i64", d)))?;
                if n > 0 {
                    ctx.emit(&[Value::Numeric(Decimal {
                        unscaled: n as i128,
                        scale: 0,
                    })])?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
```

```sql
CREATE OR REPLACE RUST SET SCRIPT my_schema.set_filter(x BIGINT) EMITS (y BIGINT) AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/

SELECT my_schema.set_filter(x) FROM nums;  -- one row per positive input value
```

`set_filter` emits zero or one row per input row it sees, so a group of N rows can produce anywhere from 0 to N output rows. See `test-udfs/set-filter` for the full fixture.

## 11. Emitting Arrow batches (SET EMITS via `RecordBatch`)

§10 emits one `Value` row at a time. This is the same SET EMITS output shape, but through a batch-oriented method: if your UDF data is already in an Arrow `RecordBatch`, you can emit it directly without converting each row to `Value`. The runtime encodes the batch column-by-column according to the declared `EMITS` schema and applies the same 4 MB flush semantics as row-based `emit`.

### Enable the feature

```toml
[dependencies]
exasol-udf-sdk = { version = "0.11", features = ["emit-arrow"] }
arrow = "58"
```

The `connect-back` feature implies `emit-arrow`, so you do not need to list it separately if you already enable `connect-back`.

### The method

```rust
fn emit_batch(&mut self, batch: &RecordBatch) -> Result<(), UdfError>;
```

The declared `EMITS` schema — not the Arrow schema — dictates each column's Exasol type. If a column's Arrow type cannot be converted to the declared Exasol type, `emit_batch` returns `UdfError::Type`.

### Example

```rust
use std::sync::Arc;

use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

#[exasol_udf]
pub fn emit_from_arrow(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}  // drain input rows (SET UDF)

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let ids    = Arc::new(Int64Array::from(vec![1i64, 2, 3]));
    let labels = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    let batch  = RecordBatch::try_new(schema, vec![ids, labels])
        .map_err(|e| UdfError::User(e.to_string()))?;

    ctx.emit_batch(&batch)
}
```

```sql
CREATE OR REPLACE RUST SET SCRIPT my_schema.emit_from_arrow(dummy BOOLEAN)
EMITS (id BIGINT, label VARCHAR(1)) AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

## 12. Connect-back

Connect-back lets a UDF open a regular Exasol connection from inside `run()` and execute SQL. The connect-back session is an ordinary independent SQL login — Exasol treats it exactly like a connection from PyExasol, JDBC, or any other external client. It has its own session and its own transaction; it cannot access the invoking query's transaction.

### Enable the feature

```toml
[dependencies]
exasol-udf-sdk = { version = "0.11", features = ["connect-back"] }
```

### Create a CONNECTION object

Store the credentials the UDF will use:

```sql
CREATE CONNECTION CB_SELF
  TO '<node-ip>:8563'
  USER 'sys'
  IDENTIFIED BY 'exasol';
```

For a multi-node cluster, omit the static IP and use `ctx.cluster_ip()` inside the UDF to discover the current node's address at runtime (see below). For single-node Docker, the container's eth0 address is also reachable this way.

### Declare the connection in the script

The `%connection` directive makes the named credentials available to the UDF:

```sql
CREATE OR REPLACE RUST SET SCRIPT my_schema.my_udf(...)
EMITS (...) AS
%connection CB_SELF
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

> Connect-back works from both `SCALAR` and `SET` scripts. Use whichever UDF type fits your logic. The address rules (`cluster_ip()`, never loopback) and transaction-conflict rules below apply equally to both.

### Three methods on `UdfContext`

```rust
// The current node's primary non-loopback IPv4 (e.g. eth0).
// Use this as the host in the CONNECTION object on real clusters.
fn cluster_ip(&self) -> Result<String, UdfError>;

// Fetch the named CONNECTION object's address/user/password via a
// round-trip to the database node (ZMQ MT_IMPORT).
fn connection(&self, name: &str) -> Result<ConnectionObject, UdfError>;

// Open a new, independent Exasol session using those credentials.
fn connect_back(&mut self, conn: &ConnectionObject) -> Result<Box<dyn ExaConnection>, UdfError>;
```

### `ExaConnection` API

```rust
// Execute a query and return rows as SDK Value enums — FFI-safe.
fn query(&mut self, sql: &str)   -> Result<Vec<Vec<Value>>, UdfError>;

// Execute a DML statement; return the number of affected rows.
fn execute(&mut self, sql: &str) -> Result<u64, UdfError>;

// Execute a query and return Arrow RecordBatches.
// Do NOT downcast columns in UDF code — the .so links its own copy of
// the arrow crate, so TypeIds differ across the cdylib boundary and
// downcast_ref silently returns None. Use query() instead.
fn query_arrow(&mut self, sql: &str) -> Result<Vec<RecordBatch>, UdfError>;
```

### Session model

The connect-back session autocommits by default (exarrow-rs behaviour). Each `execute()` call commits on its own. Do not call `conn.execute("COMMIT")` — there is no open transaction, so it errors after the data has already landed.

Because the connect-back session is a separate transaction, Exasol's Serializable isolation applies: if the connect-back session touches an object that the invoking query's transaction also reads or locks, the invoking transaction enters WAIT FOR COMMIT and the deadlock detector will abort it. The rules to avoid this are:

1. **Pre-create and commit the target table before the invoking query runs.** The connect-back session can only see committed objects; DDL inside the connect-back session conflicts with the invoking query's schema locks.
2. **Write a different table than the invoking query reads.** If the query reads `input_t` and the UDF inserts into `result_t`, there is no conflict. Writing the same table the query reads will collide.
3. **Do not issue DDL from within the connect-back session.** Only DML against pre-committed objects.

### Example: read data

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn db_version(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    while ctx.next()? {}  // SET only: drain remaining input rows before opening the session

    let cred = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&cred)?;
    let rows = conn.query("SELECT PARAM_VALUE FROM SYS.EXA_METADATA WHERE PARAM_NAME = 'databaseProductVersion'")?;

    let version = match rows.first().and_then(|r| r.first()) {
        Some(Value::String(s)) => s.clone(),
        _ => return Err(UdfError::User("unexpected result".into())),
    };
    Ok(Some(Value::String(version)))
}
```

For SET UDFs, drain all remaining input rows before opening the connect-back session. For SCALAR UDFs the single input row is pre-loaded and there is nothing to drain. In both cases, do not interleave ZMQ control traffic with the connect-back TCP session — they share the same thread and interleaving causes a deadlock.

### Example: write data

Pre-create the target table in a separate session before running the query, then insert from the UDF:

```sql
-- Run once before the query:
CREATE OR REPLACE TABLE my_schema.results (v BIGINT, v_squared BIGINT);
```

```rust
#[exasol_udf]
pub fn compute_and_store(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // Drain input before opening the session.
    let mut vals: Vec<i64> = Vec::new();
    while ctx.next()? {
        if let Some(n) = ctx.get_i64(0)? {
            vals.push(n);
        }
    }

    let cred = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&cred)?;

    // Batch all rows into one INSERT — each execute() autocommits.
    if !vals.is_empty() {
        let rows = vals.iter()
            .map(|v| format!("({v}, {})", v * v))
            .collect::<Vec<_>>()
            .join(", ");
        conn.execute(&format!("INSERT INTO my_schema.results VALUES {rows}"))?;
    }

    // BIGINT output column → emit as Numeric.
    for _ in &vals {
        ctx.emit(&[Value::Numeric(Decimal::from(1_i64))])?;
    }
    Ok(())
}
```

Calling this from SQL (`SELECT compute_and_store(v) EMITS (status BIGINT) FROM input_t`) writes squared pairs into `my_schema.results` while returning a status value per input row.

### Discovering the node IP at runtime

On a real cluster the node IP changes per deployment. Read it at runtime rather than hardcoding it:

```rust
#[exasol_udf]
pub fn node_ip(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    let ip = ctx.cluster_ip()?;
    Ok(Some(Value::String(ip)))
}
```

Use the returned IP when constructing the `CONNECTION` object, or store it in a table as part of cluster administration.

## 13. Build and deploy

```bash
# Cross-compile to a musl .so (release profile, stripped)
cargo exasol-udf build

# Artifact:
#   target/x86_64-unknown-linux-musl/release/libmy_udf.so

# Upload to BucketFS via the HTTP API or your admin tooling, then register:
```

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.scalar_double(val BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

`cargo exasol-udf build` is equivalent to `cargo build --target x86_64-unknown-linux-musl --release`; it sets the correct target and profile without requiring you to remember the flags.

## 14. Unit testing

`UdfContext` is a trait. Implement a stub to test UDF logic without a live cluster. Test a RETURNS UDF by asserting on the function's own `Result<Option<T>, UdfError>` return value — the mock does not need to record an output side channel, and should return `Err` from `emit()` so an accidental author `emit()` call fails the test loudly. Test an EMITS UDF by recording into an `emitted: Vec<Vec<Value>>` field and asserting on that.

### RETURNS: `scalar_double`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        input: Vec<Value>,
    }

    impl TestCtx {
        fn new(row: Vec<Value>) -> Self {
            Self { input: row }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize {
            self.input.len()
        }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.input
                .get(col)
                .ok_or_else(|| UdfError::User(format!("col {} out of range", col)))
        }

        fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
            Err(UdfError::Unimplemented("emit is banned in RETURNS output".into()))
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
    }

    #[test]
    fn doubles_positive_int64() {
        let mut ctx = TestCtx::new(vec![Value::Int64(21)]);
        let result = scalar_double(&mut ctx).unwrap();
        assert_eq!(result, Some(Value::Int64(42)));
    }

    #[test]
    fn passes_null_through() {
        let mut ctx = TestCtx::new(vec![Value::Null]);
        let result = scalar_double(&mut ctx).unwrap();
        assert_eq!(result, None);
    }
}
```

The mock does not override `set_return` — the default (`Err(UdfError::Unimplemented(...))`) is never exercised, because the test asserts on `scalar_double`'s own return value, not on a side channel. `set_return` only matters to the runtime bridge driving the compiled `.so`; a unit test calls the annotated function directly.

### EMITS: `set_filter`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        rows: Vec<Vec<Value>>,
        cursor: usize,
        emitted: Vec<Vec<Value>>,
    }

    impl TestCtx {
        fn new(rows: Vec<Vec<Value>>) -> Self {
            Self { rows, cursor: 0, emitted: Vec::new() }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize {
            self.rows.first().map_or(0, |r| r.len())
        }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.rows[self.cursor - 1]
                .get(col)
                .ok_or_else(|| UdfError::User(format!("col {} out of range", col)))
        }

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            if self.cursor < self.rows.len() {
                self.cursor += 1;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    #[test]
    fn emits_only_positive_rows() {
        let mut ctx = TestCtx::new(vec![
            vec![Value::Int64(-1)],
            vec![Value::Int64(3)],
        ]);
        set_filter(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Int64(3)]]);
    }
}
```

For connect-back UDFs, stub `cluster_ip`, `connection`, and `connect_back` with test implementations that return canned data, or use the integration tests in `crates/it` against a real Docker container.
