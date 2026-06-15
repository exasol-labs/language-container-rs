[language-container-rs](../README.md) › [docs](index.md) › Writing a Rust UDF

---

# Writing a Rust UDF

## Prerequisites

- Rust 1.92+ with the musl target:
  ```bash
  rustup target add x86_64-unknown-linux-musl
  ```
- `cargo-exaudf` installed from this workspace:
  ```bash
  cargo install --path crates/cargo-exaudf
  ```
- A running Exasol cluster with BucketFS write access.

## 1. Scaffold a UDF crate

```bash
cargo exaudf new my-udf
cd my-udf
```

Or create the crate manually. The crate must be a `cdylib`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
exasol-udf-sdk    = { version = "0.8" }
exasol-udf-macros = { version = "0.8" }
```

## 2. The `#[exasol_udf]` macro

Annotate a public function with `#[exasol_udf]`. The macro generates the C ABI entry point and vtable the runtime expects.

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn my_udf(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // ...
    Ok(())
}
```

The function name becomes the SQL script name (case-insensitive). Return `Ok(())` after all `emit` calls are done.

### Optional type annotations

If you annotate the input and output types, the runtime validates the SQL column schema at load time:

```rust
#[exasol_udf(input(val: i64), emits(result: i64))]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // ...
}
```

Supported annotation types: `i32`, `i64`, `f64`, `f32`, `bool`, `String`, `&str`, `Decimal`, `NaiveDate`, `NaiveDateTime`.

## 3. The `UdfContext` interface

Every UDF receives `&mut dyn UdfContext`. The three core operations are:

| Method | What it does |
|--------|-------------|
| `ctx.get(col)` | Returns `&Value` for column `col` (0-indexed) on the current input row |
| `ctx.emit(values)` | Appends one output row |
| `ctx.next()` | Advances to the next input row; returns `false` when exhausted |

`next()` is for SET UDFs only — call it before the first `get()` on each row. Scalar UDFs start with the single input row already loaded.

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

`ExaType` is the column-level SQL type, independent of the wire value. It is available via `ctx.column_type(col)` (returns `UdfError::Unimplemented` if the runtime does not populate it). The full variant set covers `Double`, `Int32`, `Int64`, `Numeric { precision, scale }`, `Boolean`, `String { size }`, `Char { size }`, `Date`, `Timestamp`, `TimestampTz`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, and `Unsupported`.

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

## 6. Scalar UDF example

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let doubled = match ctx.get_i64(0)? {
        // get_i64 handles both Int64 and scale-0 Numeric (BIGINT).
        Some(n) => Value::Numeric(Decimal::from(n * 2)),
        None    => Value::Null,
    };
    ctx.emit(&[doubled])
}
```

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.scalar_double(val BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/

SELECT my_schema.scalar_double(21);  -- 42
```

## 7. SET UDF example

SET UDFs receive every row in a group. Call `ctx.next()` before each `ctx.get()`.

```rust
#[exasol_udf]
pub fn set_sum(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut total: i64 = 0;
    while ctx.next()? {
        if let Some(n) = ctx.get_i64(0)? {
            total += n;
        }
    }
    ctx.emit(&[Value::Numeric(Decimal::from(total))])
}
```

```sql
CREATE OR REPLACE RUST SET SCRIPT my_schema.set_sum(val BIGINT)
EMITS (result BIGINT) AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/

SELECT my_schema.set_sum(v) EMITS (result BIGINT) FROM my_table;
```

## 8. Connect-back

Connect-back lets a UDF open a regular Exasol connection from inside `run()` and execute SQL. The connect-back session is an ordinary independent SQL login — Exasol treats it exactly like a connection from PyExasol, JDBC, or any other external client. It has its own session and its own transaction; it cannot access the invoking query's transaction.

### Enable the feature

```toml
[dependencies]
exasol-udf-sdk = { version = "0.8", features = ["connect-back"] }
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

> **Always use `SET SCRIPT ... EMITS (...)` for connect-back UDFs.** A `SCALAR SCRIPT ... RETURNS ...` UDF crashes the SQL worker process when it opens a connect-back session.

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
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn db_version(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}  // drain input before opening the connection

    let cred = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&cred)?;
    let rows = conn.query("SELECT PARAM_VALUE FROM SYS.EXA_METADATA WHERE PARAM_NAME = 'databaseProductVersion'")?;

    let version = match rows.first().and_then(|r| r.first()) {
        Some(Value::String(s)) => s.clone(),
        _ => return Err(UdfError::User("unexpected result".into())),
    };
    ctx.emit(&[Value::String(version)])
}
```

Drain all input rows before opening the connect-back session. The ZMQ control channel and the TCP session share the same thread; interleaving them causes a deadlock.

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
pub fn node_ip(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let ip = ctx.cluster_ip()?;
    ctx.emit(&[Value::String(ip)])
}
```

Use the returned IP when constructing the `CONNECTION` object, or store it in a table as part of cluster administration.

## 9. Build and deploy

```bash
# Cross-compile to a musl .so (release profile, stripped)
cargo exaudf build

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

`cargo exaudf build` is equivalent to `cargo build --target x86_64-unknown-linux-musl --release`; it sets the correct target and profile without requiring you to remember the flags.

## 10. Unit testing

`UdfContext` is a trait. Implement a stub to test UDF logic without a live cluster:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        rows:    Vec<Vec<Value>>,
        cursor:  usize,
        emitted: Vec<Vec<Value>>,
    }

    impl TestCtx {
        fn new(rows: Vec<Vec<Value>>) -> Self {
            // Start one before the first row so next() advances into row 0.
            Self { rows, cursor: usize::MAX, emitted: Vec::new() }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize {
            self.rows.first().map_or(0, |r| r.len())
        }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.rows[self.cursor]
                .get(col)
                .ok_or_else(|| UdfError::User(format!("col {col} out of range")))
        }

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            self.cursor = self.cursor.wrapping_add(1);
            Ok(self.cursor < self.rows.len())
        }
    }

    #[test]
    fn set_sum_over_three_rows() {
        let mut ctx = TestCtx::new(vec![
            vec![Value::Int64(1)],
            vec![Value::Int64(2)],
            vec![Value::Int64(3)],
        ]);
        set_sum(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted,
            vec![vec![Value::Numeric(Decimal::from(6_i64))]]
        );
    }

    #[test]
    fn scalar_double_null_passthrough() {
        // For scalar UDFs the row is pre-loaded; next() is not called.
        let mut ctx = TestCtx::new(vec![vec![Value::Null]]);
        ctx.cursor = 0;  // point directly at the single row
        scalar_double(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Null]]);
    }
}
```

For connect-back UDFs, stub `cluster_ip`, `connection`, and `connect_back` with test implementations that return canned data, or use the integration tests in `crates/it` against a real Docker container.
