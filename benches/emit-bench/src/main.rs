//! Emit-throughput benchmark: Rust SLC vs native Python3.
//!
//! Boots Exasol (Docker via `it::Harness`, or external mode via env vars),
//! registers both the Rust SLC and the builtin Python3, then measures three
//! segments per cell — SLC startup, data generation, and **data transfer** (the
//! headline metric) — across the mixed shape `id BIGINT, label VARCHAR(100),
//! val DOUBLE`, in row and columnar modes, at 1M and 5M rows.
//!
//! Each UDF takes `(n, do_emit)`: `do_emit=0` builds all N rows but emits one
//! sentinel (generation), `do_emit=1` emits all N (full). Transfer = full − gen.
//!
//! Run: see benches/README.md. Requires `SLC_TARBALL` to point at the built
//! container tarball.

use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use exarrow_rs::adbc::Connection;
use it::{Harness, query_single_string, read_udf_artifact};

const UDF_LIB: &str = "libemit_bench_udf.so";
/// id (8) + label (50) + val (8). Used only for the MB/s figure.
const BYTES_PER_ROW: f64 = 66.0;
const RUNS: usize = 5;
const WARMUP_N: i64 = 100_000;
const NS: [i64; 2] = [1_000_000, 5_000_000];

#[tokio::main]
async fn main() -> Result<()> {
    let harness = Harness::start().await?;

    // Readiness wait via exapump (falls back to the connect retry if absent).
    wait_ready_exapump(&harness.host, harness.db_port).await;

    let mut conn = harness.connect().await?;

    // Upload the SLC tarball into BucketFS and assemble a SCRIPT_LANGUAGES that
    // keeps the builtin languages (PYTHON3, ...) AND adds RUST. Reading the
    // system default avoids hardcoding the Python3 token.
    let slc = harness.load_slc().await?;
    let system_langs = query_single_string(
        &mut conn,
        "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'",
    )
    .await?
    .ok_or_else(|| anyhow!("could not read system SCRIPT_LANGUAGES"))?;
    let combined = format!("{} {}", system_langs, slc.script_languages());
    conn.execute(&format!("ALTER SESSION SET SCRIPT_LANGUAGES='{combined}'"))
        .await
        .context("setting combined SCRIPT_LANGUAGES")?;

    // Upload the Rust UDF .so and create all scripts.
    let udf_path = harness
        .upload_udf(UDF_LIB, read_udf_artifact(UDF_LIB)?)
        .await?;
    conn.execute("CREATE SCHEMA IF NOT EXISTS bench").await.ok();
    create_scripts(&mut conn, &udf_path).await?;

    let pandas = pandas_available(&mut conn).await;
    if !pandas {
        eprintln!(
            "[emit-bench] pandas not available in builtin Python3 — columnar Python cells = N/A"
        );
    }

    // Cold-start probes FIRST (before any warmup), at N=1: Rust process spawn +
    // dlopen, Python interpreter + base import.
    let (startup_rust, _) =
        time_query(&mut conn, "SELECT bench.emit_mixed_row(1, 1) FROM DUAL").await?;
    let (startup_py, _) =
        time_query(&mut conn, "SELECT bench.py_mixed_row(1, 1) FROM DUAL").await?;

    // Matrix: 2 modes × 2 langs × 2 N.
    let modes = [
        ("row", "bench.emit_mixed_row", "bench.py_mixed_row"),
        ("batch", "bench.emit_mixed_batch", "bench.py_mixed_batch"),
    ];
    let mut results: Vec<Cell> = Vec::new();
    for (mode, rust_script, py_script) in modes {
        for &n in &NS {
            results.push(run_cell(&mut conn, mode, "rust", rust_script, n).await?);
            if pandas || mode == "row" {
                results.push(run_cell(&mut conn, mode, "python3", py_script, n).await?);
            } else {
                results.push(Cell::na(mode, "python3", n));
            }
        }
    }

    print_report(startup_rust, startup_py, pandas, &results);
    Ok(())
}

/// One measured matrix cell.
struct Cell {
    mode: &'static str,
    lang: &'static str,
    n: i64,
    gen_s: f64,
    full_s: f64,
    xfer_s: f64,
    rows_per_s: f64,
    mb_per_s: f64,
    na: bool,
}

impl Cell {
    fn na(mode: &'static str, lang: &'static str, n: i64) -> Self {
        Cell {
            mode,
            lang,
            n,
            gen_s: 0.0,
            full_s: 0.0,
            xfer_s: 0.0,
            rows_per_s: 0.0,
            mb_per_s: 0.0,
            na: true,
        }
    }
}

async fn run_cell(
    conn: &mut Connection,
    mode: &'static str,
    lang: &'static str,
    script: &str,
    n: i64,
) -> Result<Cell> {
    // Warmup (discarded).
    let _ = time_query(conn, &full_sql(script, WARMUP_N)).await?;

    // T_full — emit and fetch all N rows; verify the count before trusting timing.
    let mut fulls = Vec::with_capacity(RUNS);
    let mut rows_seen = 0usize;
    for _ in 0..RUNS {
        let (d, rows) = time_query(conn, &full_sql(script, n)).await?;
        fulls.push(d);
        rows_seen = rows;
    }
    if rows_seen as i64 != n {
        bail!("{lang}/{mode} N={n}: emitted {rows_seen} rows, expected {n}");
    }

    // T_generation — build all N rows, emit one sentinel.
    let mut gens = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let (d, _) = time_query(conn, &gen_sql(script, n)).await?;
        gens.push(d);
    }

    let full_s = secs(median(fulls));
    let gen_s = secs(median(gens));
    let xfer_s = (full_s - gen_s).max(1e-6); // clamp against measurement noise
    let rows_per_s = n as f64 / xfer_s;
    let mb_per_s = n as f64 * BYTES_PER_ROW / 1e6 / xfer_s;
    Ok(Cell {
        mode,
        lang,
        n,
        gen_s,
        full_s,
        xfer_s,
        rows_per_s,
        mb_per_s,
        na: false,
    })
}

fn full_sql(script: &str, n: i64) -> String {
    format!("SELECT {script}({n}, 1) FROM DUAL")
}
fn gen_sql(script: &str, n: i64) -> String {
    format!("SELECT {script}({n}, 0) FROM DUAL")
}

/// Time a query and return (elapsed, total rows fetched).
async fn time_query(conn: &mut Connection, sql: &str) -> Result<(Duration, usize)> {
    let t = Instant::now();
    let batches = conn
        .query(sql)
        .await
        .with_context(|| format!("query: {sql}"))?;
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    Ok((t.elapsed(), rows))
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}
fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

async fn create_scripts(conn: &mut Connection, udf_path: &str) -> Result<()> {
    for name in ["emit_mixed_row", "emit_mixed_batch"] {
        conn.execute(&format!(
            "CREATE OR REPLACE RUST SET SCRIPT bench.{name}(n BIGINT, do_emit BIGINT) \
             EMITS (id BIGINT, label VARCHAR(100), val DOUBLE) AS\n\
             %udf_object {udf_path};\n/"
        ))
        .await
        .with_context(|| format!("create rust script {name}"))?;
    }

    // Python3 row script. LABEL is 50 chars to match the Rust side.
    //
    // NOTE: raw string, body left-aligned at column 0 — a backslash-continued
    // normal string would strip Python's leading indentation and break it.
    conn.execute(
        r#"CREATE OR REPLACE PYTHON3 SET SCRIPT bench.py_mixed_row(n BIGINT, do_emit BIGINT) EMITS (id BIGINT, label VARCHAR(100), val DOUBLE) AS
LABEL = '0123456789012345678901234567890123456789012345678'
def run(ctx):
    n = ctx.n
    do_emit = ctx.do_emit
    if do_emit:
        for i in range(n):
            ctx.emit(i, LABEL, i * 1.5)
    else:
        for i in range(n):
            row = (i, LABEL, i * 1.5)
        ctx.emit(0, LABEL, 0.0)
/"#,
    )
    .await
    .context("create py_mixed_row")?;

    // Python3 columnar script (pandas DataFrame emit).
    conn.execute(
        r#"CREATE OR REPLACE PYTHON3 SET SCRIPT bench.py_mixed_batch(n BIGINT, do_emit BIGINT) EMITS (id BIGINT, label VARCHAR(100), val DOUBLE) AS
import pandas as pd
import numpy as np
LABEL = '0123456789012345678901234567890123456789012345678'
CHUNK = 100000
def run(ctx):
    n = ctx.n
    do_emit = ctx.do_emit
    emitted = 0
    while emitted < n:
        ln = min(CHUNK, n - emitted)
        ids = np.arange(emitted, emitted + ln, dtype='int64')
        df = pd.DataFrame({'id': ids, 'label': [LABEL] * ln, 'val': ids * 1.5})
        if do_emit:
            ctx.emit(df)
        emitted += ln
    if not do_emit:
        ctx.emit(pd.DataFrame({'id': [0], 'label': [LABEL], 'val': [0.0]}))
/"#,
    )
    .await
    .context("create py_mixed_batch")?;

    Ok(())
}

/// Probe whether the builtin Python3 has pandas (needed for columnar emit).
async fn pandas_available(conn: &mut Connection) -> bool {
    let create = conn
        .execute(
            r#"CREATE OR REPLACE PYTHON3 SCALAR SCRIPT bench.py_has_pandas() RETURNS VARCHAR(10) AS
def run(ctx):
    try:
        import pandas
        return 'ok'
    except Exception:
        return 'no'
/"#,
        )
        .await;
    if create.is_err() {
        return false;
    }
    matches!(
        query_single_string(conn, "SELECT bench.py_has_pandas()").await,
        Ok(Some(s)) if s == "ok"
    )
}

/// Poll `exapump sql ... "SELECT 1"` until the DB answers. If exapump is not
/// installed, return quietly — `connect()` retries on its own afterwards.
async fn wait_ready_exapump(host: &str, port: u16) {
    let dsn = format!("exasol://sys:exasol@{host}:{port}/?validateservercertificate=0");
    for attempt in 1..=60 {
        let dsn = dsn.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            Command::new("exapump")
                .args(["sql", "--dsn", &dsn, "SELECT 1"])
                .output()
        })
        .await
        .expect("spawn_blocking");
        match outcome {
            Ok(o) if o.status.success() => {
                eprintln!("[emit-bench] Exasol ready (exapump probe).");
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("[emit-bench] exapump not found; relying on connect() retry.");
                return;
            }
            _ => {
                eprintln!("[emit-bench] exapump probe {attempt}/60 not ready; retry in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    eprintln!("[emit-bench] exapump never reported ready; continuing to connect() anyway.");
}

fn print_report(startup_rust: Duration, startup_py: Duration, pandas: bool, cells: &[Cell]) {
    println!("\n=== emit-throughput: Rust SLC vs native Python3 ===");
    println!("shape: id BIGINT, label VARCHAR(100), val DOUBLE   (~{BYTES_PER_ROW:.0} B/row)");
    println!("metric: data transfer = T_full - T_generation (median of {RUNS} runs)\n");

    println!("SLC startup (cold first call, N=1):");
    println!("  rust    : {:>7.1} ms", startup_rust.as_secs_f64() * 1e3);
    println!("  python3 : {:>7.1} ms", startup_py.as_secs_f64() * 1e3);
    if !pandas {
        println!("  (python3 columnar = N/A: pandas absent)");
    }

    println!(
        "\n{:<6} {:<8} {:>10} {:>9} {:>9} {:>9} {:>13} {:>10}",
        "mode", "lang", "N", "gen(s)", "full(s)", "xfer(s)", "rows/s", "MB/s"
    );
    println!("{}", "-".repeat(82));
    for c in cells {
        if c.na {
            println!(
                "{:<6} {:<8} {:>10} {:>9} {:>9} {:>9} {:>13} {:>10}",
                c.mode, c.lang, c.n, "-", "-", "-", "N/A", "N/A"
            );
            continue;
        }
        println!(
            "{:<6} {:<8} {:>10} {:>9.3} {:>9.3} {:>9.3} {:>13.0} {:>10.1}",
            c.mode, c.lang, c.n, c.gen_s, c.full_s, c.xfer_s, c.rows_per_s, c.mb_per_s
        );
    }

    // Rust-vs-Python transfer ratio per (mode, N).
    println!("\ntransfer throughput ratio (rust rows/s ÷ python3 rows/s):");
    for mode in ["row", "batch"] {
        for &n in &NS {
            let r = cells
                .iter()
                .find(|c| c.mode == mode && c.lang == "rust" && c.n == n && !c.na);
            let p = cells
                .iter()
                .find(|c| c.mode == mode && c.lang == "python3" && c.n == n && !c.na);
            match (r, p) {
                (Some(r), Some(p)) if p.rows_per_s > 0.0 => {
                    println!("  {mode:<6} N={n:<9} {:.2}x", r.rows_per_s / p.rows_per_s)
                }
                _ => println!("  {mode:<6} N={n:<9} N/A"),
            }
        }
    }
}
