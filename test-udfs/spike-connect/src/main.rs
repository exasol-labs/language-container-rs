//! Spike: verify exarrow-rs can connect to an Exasol address as a plain
//! external client, run SELECT 42, and return the result.
//!
//! Usage: spike-connect <host:port> [user] [password]
//!   host:port  — Exasol SQL endpoint (e.g. 172.17.0.2:8563)
//!   user       — defaults to "sys"
//!   password   — defaults to "exasol"
//!
//! Exit 0 on success, 1 on any failure. Prints the result row to stdout.
use anyhow::{Context, Result, anyhow};
use arrow::array::Int64Array;
use exarrow_rs::adbc::{Connection, Driver};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: spike-connect <host:port> [user] [password]");
        std::process::exit(1);
    }
    let addr = &args[1];
    let user = args.get(2).map(String::as_str).unwrap_or("sys");
    let password = args.get(3).map(String::as_str).unwrap_or("exasol");

    match run(addr, user, password).await {
        Ok(val) => {
            println!("OK: SELECT 42 => {val}");
        }
        Err(e) => {
            eprintln!("FAIL: {e:#}");
            std::process::exit(1);
        }
    }
}

async fn run(addr: &str, user: &str, password: &str) -> Result<i64> {
    let dsn = format!("exasol://{user}:{password}@{addr}?validateservercertificate=0");
    eprintln!("connecting to {addr} (native protocol, no transport override)");

    let driver = Driver::new();
    let db = driver.open(&dsn).context("driver.open")?;
    let mut conn: Connection = db.connect().await.context("db.connect")?;
    eprintln!("connected; running SELECT 42");

    let batches = conn.query("SELECT 42").await.context("query")?;
    let batch = batches.first().ok_or_else(|| anyhow!("no result batch"))?;
    let col = batch.column(0);
    eprintln!(
        "result column type: {:?}, rows: {}",
        col.data_type(),
        batch.num_rows()
    );

    // Exasol returns integer literals as Decimal128; cast to i128 then i64.
    use arrow::array::Decimal128Array;
    let val = if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
        a.value(0)
    } else if let Some(a) = col.as_any().downcast_ref::<Decimal128Array>() {
        a.value(0) as i64
    } else {
        return Err(anyhow!("unexpected column type: {:?}", col.data_type()));
    };

    conn.close().await.ok();
    Ok(val)
}
