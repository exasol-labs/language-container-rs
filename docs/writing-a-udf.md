[language-container-rs](../README.md) › [docs](index.md) › Writing a Rust UDF

---

# Writing a Rust UDF

## Prerequisites

- Rust 1.84+ with the musl target:
  ```bash
  rustup target add x86_64-unknown-linux-musl
  ```
- `cargo-exaudf` installed from this workspace:
  ```bash
  cargo install --path crates/cargo-exaudf
  ```
- A running Exasol cluster with BucketFS write access.

## 1. Create the crate

```bash
cargo exaudf new my-udf
cd my-udf
```

Or scaffold manually. The crate must be a `cdylib`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
exasol-udf-sdk    = { version = "0.7" }
exasol-udf-macros = { version = "0.7" }
```

## 2. Implement the UDF

Annotate a public function with `#[exasol_udf]`. The macro generates the C ABI entry point the runtime expects.

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // get_i64 accepts both Value::Int64 and a scale-0 Value::Numeric, so a
    // column typed BIGINT (which arrives as Numeric, see below) just works.
    let doubled = match ctx.get_i64(0)? {
        Some(n) => Value::Int64(n * 2),
        None    => Value::Null,
    };
    ctx.emit(&[doubled])
}
```

The function name becomes the SQL script name (case-insensitive). Return `Ok(())` after all `emit` calls are done.

## 3. Using input values

`ctx.get(col)` returns `&Value` for the given 0-indexed column on the current input row.

| Variant | SQL type |
|---------|----------|
| `Value::Int64(i64)` | BIGINT |
| `Value::Float64(f64)` | DOUBLE |
| `Value::String(String)` | VARCHAR / CHAR |
| `Value::Bool(bool)` | BOOLEAN |
| `Value::Numeric(Decimal)` | DECIMAL |
| `Value::Date(NaiveDate)` | DATE |
| `Value::Timestamp(NaiveDateTime)` | TIMESTAMP |
| `Value::Null` | NULL |

`Numeric`, `Date`, and `Timestamp` are now strongly typed rather than raw strings. `Decimal` is a `{ unscaled: i128, scale: u8 }` newtype with 38-digit precision and no allocation: build it with `Decimal::from_str("3.14")` or `Decimal::from_f64(3.14)`, do arithmetic via its `.unscaled` / `.scale` fields, and render it with `to_string()` (`"3.14"`). `Date` and `Timestamp` carry `chrono::NaiveDate` / `chrono::NaiveDateTime`.

> **Note:** Exasol sends `BIGINT` columns over the wire as `PB_NUMERIC`, so a column typed `BIGINT` arrives as `Value::Numeric` (with `scale == 0`), not `Value::Int64`. Prefer the typed getter `ctx.get_i64()` below — it accepts both variants — over matching the enum by hand.

### Typed getters

In addition to `ctx.get(col) -> &Value`, `UdfContext` exposes per-type getters that unwrap the variant for you and return `Ok(None)` for SQL `NULL`:

```rust
fn get_i64(&self, col: usize)     -> Result<Option<i64>,           UdfError>;
fn get_f64(&self, col: usize)     -> Result<Option<f64>,           UdfError>;
fn get_str(&self, col: usize)     -> Result<Option<&str>,          UdfError>;
fn get_bool(&self, col: usize)    -> Result<Option<bool>,          UdfError>;
fn get_decimal(&self, col: usize) -> Result<Option<Decimal>,       UdfError>;
fn get_date(&self, col: usize)    -> Result<Option<NaiveDate>,     UdfError>;
fn get_datetime(&self, col: usize)-> Result<Option<NaiveDateTime>, UdfError>;
```

Each returns a type error if the column holds a different variant — except `get_i64`, which deliberately also accepts a `Value::Numeric` whose `scale == 0` (the BIGINT-as-`PB_NUMERIC` reality above). Reach for these instead of matching `Value` by hand whenever you want a single concrete type.

### Set UDFs

For `SET` (multi-row) UDFs, loop with `ctx.next()` before reading each row:

```rust
#[exasol_udf]
pub fn set_filter(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {
        match ctx.get(0)? {
            Value::Int64(n) if *n > 0 => ctx.emit(&[Value::Int64(*n)])?,
            _ => {}
        }
    }
    Ok(())
}
```

`ctx.next()` returns `false` when there are no more rows. Call it before the first `ctx.get()`.

## 4. Connect-back

Connect-back lets a UDF open a new Exasol session and run SQL from inside `run()`. It is an opt-in feature.

### Cargo.toml

```toml
[dependencies]
exasol-udf-sdk = { version = "0.7", features = ["connect-back"] }
```

### Operator setup

Create a `CONNECTION` object the UDF can reference:

```sql
CREATE CONNECTION CB_SELF
  TO 'cluster-host:8563'
  USER 'sys'
  IDENTIFIED BY 'exasol';
```

For a single-node Docker cluster, `cluster-host` is the container's IP. On a real cluster, use `ctx.cluster_ip()` in the UDF to discover the node IP at runtime — no static address needed.

### Script definition

The `%connection` directive tells the runtime which credentials to make available:

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.my_udf() RETURNS VARCHAR(100) AS
%connection CB_SELF
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

### Three connect-back methods on `UdfContext`

```rust
// Reads the local node's primary IPv4 via getifaddrs (first non-loopback
// interface, e.g. eth0); works on single-node Docker and multi-node clusters
fn cluster_ip(&self) -> Result<String, UdfError>

// Fetches credentials via ZMQ MT_IMPORT (one round-trip to the DB node)
fn connection(&self, name: &str) -> Result<ConnectionObject, UdfError>

// Opens a new ADBC session; returns a live connection
fn connect_back(&mut self, conn: &ConnectionObject) -> Result<Box<dyn ExaConnection>, UdfError>
```

### Example: emit the cluster node IP

```rust
#[exasol_udf]
pub fn connect_back_cluster_ip(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let ip = ctx.cluster_ip()?;
    ctx.emit(&[Value::String(ip)])
}
```

### Example: query the database and return a value

Use `query()`, which returns rows of the SDK `Value` enum:

```rust
#[exasol_udf]
pub fn connect_back_query(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let c = ctx.connection("CB_SELF")?;
    let rows = ctx.connect_back(&c)?.query("SELECT CAST(42 AS BIGINT)")?;
    let n: i64 = match rows.first().and_then(|r| r.first()) {
        // BIGINT/DECIMAL arrive as Value::Numeric (a scale-0 Decimal here).
        Some(Value::Numeric(d)) => d.unscaled as i64,
        Some(Value::Int64(n))   => *n,
        _ => 0,
    };
    // Emit Numeric for a BIGINT output column (see Pitfalls in the write-back guide).
    // Decimal is re-exported from exasol_udf_sdk::value.
    ctx.emit(&[Value::Numeric(Decimal::from(n))])
}
```

`ExaConnection` exposes:

```rust
// FFI-safe: converts results to the SDK Value enum inside the runtime.
fn query(&mut self, sql: &str)       -> Result<Vec<Vec<Value>>, UdfError>;
fn execute(&mut self, sql: &str)     -> Result<u64, UdfError>;
// Same-process only: returns arrow batches. Do NOT downcast these in UDF code —
// the .so links its own arrow and TypeIds differ across the FFI boundary.
fn query_arrow(&mut self, sql: &str) -> Result<Vec<RecordBatch>, UdfError>;
```

### Session semantics

`connect_back` always opens a **new external-client session** in a **new independent transaction** — a plain SQL login, exactly like any external client. The invoking query's transaction is not accessible from the UDF. Writes committed in the connect-back session are visible to other sessions once committed, independent of the outer query.

For modifying the database from a UDF, read the **[write-back guide](write-back-guide.md)** — Exasol's Serializable isolation imposes rules (pre-commit the target, no DDL in the invoking schema, autocommit, emit BIGINT as `Numeric`) that you must follow to avoid transaction-conflict aborts. For the wire protocol behind UDF execution, see **[the protocol reference](protocol.md)**.

## 5. Build and deploy

```bash
# Cross-compile to musl .so (release profile, stripped)
cargo exaudf build

# The artifact is at:
#   target/x86_64-unknown-linux-musl/release/libmy_udf.so

# Upload to BucketFS via the BucketFS HTTP API or your admin tooling, then:
```

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.double(val BIGINT) RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libmy_udf.so;
/
```

## Unit testing

`UdfContext` is a trait — implement a test stub to drive the UDF without a real Exasol cluster:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        input:   Vec<Value>,
        emitted: Vec<Vec<Value>>,
    }

    impl TestCtx {
        fn new(row: Vec<Value>) -> Self {
            Self { input: row, emitted: Vec::new() }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize { self.input.len() }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.input.get(col).ok_or_else(|| UdfError::User(format!("col {} out of range", col)))
        }

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> { Ok(false) }
    }

    #[test]
    fn doubles_positive_int64() {
        let mut ctx = TestCtx::new(vec![Value::Int64(21)]);
        scalar_double(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Int64(42)]]);
    }
}
```

For set UDFs, store multiple rows in the stub and advance `cursor` in `next()`. See `test-udfs/set-filter/src/lib.rs` for a complete example.
