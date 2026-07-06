//! Emit-throughput benchmark: Rust SLC vs native Python3.
//!
//! Boots Exasol (Docker via `it::Harness`, or external mode via env vars),
//! registers both the Rust SLC and the builtin Python3, then measures per
//! shape/mode/lang/N cell:
//!
//! - **emit** — SLC startup, data generation, and **data transfer** (the
//!   headline metric): `T_transfer = T_full − T_generation`.
//! - **ingest** (Rust only) — decode-side cost of `InputRowSet::from_proto` /
//!   `decode_string_block`: a `sink_<shape>` SET script reads every column of
//!   every row produced by `emit_<shape>_<mode>` and reports the count;
//!   `T_ingest = T_ingest_full − T_full` (the already-measured emit `T_full`
//!   for the same shape/mode/N).
//!
//! Two shapes: **mixed** (`id BIGINT, label VARCHAR(100), val DOUBLE`, no
//! string-block NUMERIC/DATE/TIMESTAMP columns) and **wide** (`id BIGINT,
//! amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, label
//! VARCHAR(100)`), in row and columnar modes, at 1M and 5M rows.
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
const RUNS: usize = 5;
const WARMUP_N: i64 = 100_000;
const NS: [i64; 2] = [1_000_000, 5_000_000];
const MODES: [&str; 2] = ["row", "batch"];

/// One benchmarked row shape: EMITS/param column DDL, the column name list
/// (same order, used to thread values through the ingest sink query), and an
/// approximate on-wire bytes/row used only for the MB/s figure.
struct Shape {
    key: &'static str,
    columns_ddl: &'static str,
    col_names: &'static str,
    bytes_per_row: f64,
}

impl Shape {
    fn rust_script(&self, mode: &str) -> String {
        format!("bench.emit_{}_{}", self.key, mode)
    }
    fn py_script(&self, mode: &str) -> String {
        format!("bench.py_{}_{}", self.key, mode)
    }
    fn sink_script(&self) -> String {
        format!("bench.sink_{}", self.key)
    }
}

const SHAPES: [Shape; 2] = [
    Shape {
        key: "mixed",
        columns_ddl: "id BIGINT, label VARCHAR(100), val DOUBLE",
        col_names: "id, label, val",
        // id (8) + label (50) + val (8).
        bytes_per_row: 66.0,
    },
    Shape {
        key: "wide",
        columns_ddl: "id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, \
                       label VARCHAR(100)",
        col_names: "id, amount, event_date, event_ts, label",
        // id (~7) + amount (~10) + event_date "YYYY-MM-DD" (10) +
        // event_ts "YYYY-MM-DD HH:MM:SS.NNNNNNNNN" (29) + label (50).
        bytes_per_row: 106.0,
    },
];

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

    // Emit matrix: shape x mode x lang x N.
    let mut results: Vec<Cell> = Vec::new();
    for shape in &SHAPES {
        for &mode in &MODES {
            let rust_script = shape.rust_script(mode);
            let py_script = shape.py_script(mode);
            for &n in &NS {
                results.push(run_cell(&mut conn, shape, mode, "rust", &rust_script, n).await?);
                if pandas || mode == "row" {
                    results.push(run_cell(&mut conn, shape, mode, "python3", &py_script, n).await?);
                } else {
                    results.push(Cell::na(shape.key, mode, "python3", n));
                }
            }
        }
    }

    // Ingest matrix (Rust only): reuses each cell's already-measured emit
    // `full_s` as the baseline to subtract.
    let mut ingest_results: Vec<IngestCell> = Vec::new();
    for shape in &SHAPES {
        for &mode in &MODES {
            for &n in &NS {
                let emit_full_s = results
                    .iter()
                    .find(|c| {
                        c.shape == shape.key
                            && c.mode == mode
                            && c.lang == "rust"
                            && c.n == n
                            && !c.na
                    })
                    .map(|c| c.full_s)
                    .ok_or_else(|| {
                        anyhow!(
                            "missing emit baseline for ingest calc: {}/{}/N={n}",
                            shape.key,
                            mode
                        )
                    })?;
                ingest_results.push(run_ingest_cell(&mut conn, shape, mode, n, emit_full_s).await?);
            }
        }
    }

    print_report(startup_rust, startup_py, pandas, &results, &ingest_results);
    Ok(())
}

/// One measured emit matrix cell.
struct Cell {
    shape: &'static str,
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
    fn na(shape: &'static str, mode: &'static str, lang: &'static str, n: i64) -> Self {
        Cell {
            shape,
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

/// One measured ingest matrix cell (Rust only).
struct IngestCell {
    shape: &'static str,
    mode: &'static str,
    n: i64,
    ingest_full_s: f64,
    ingest_only_s: f64,
    rows_per_s: f64,
    mb_per_s: f64,
}

async fn run_cell(
    conn: &mut Connection,
    shape: &'static Shape,
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
        bail!(
            "{lang}/{}/{mode} N={n}: emitted {rows_seen} rows, expected {n}",
            shape.key
        );
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
    let mb_per_s = n as f64 * shape.bytes_per_row / 1e6 / xfer_s;
    Ok(Cell {
        shape: shape.key,
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

/// Time the ingest chain `sink_<shape>(emit_<shape>_<mode>(n, 1))` and
/// subtract the already-measured emit `T_full` for the same shape/mode/N.
async fn run_ingest_cell(
    conn: &mut Connection,
    shape: &'static Shape,
    mode: &'static str,
    n: i64,
    emit_full_s: f64,
) -> Result<IngestCell> {
    let gen_script = shape.rust_script(mode);
    let sink_script = shape.sink_script();
    let sql = |n: i64| ingest_sql(&gen_script, &sink_script, shape.col_names, n);

    // Warmup (discarded).
    let _ = time_ingest_query(conn, &sql(WARMUP_N)).await?;

    let mut fulls = Vec::with_capacity(RUNS);
    let mut last_count = 0i64;
    for _ in 0..RUNS {
        let (d, count) = time_ingest_query(conn, &sql(n)).await?;
        fulls.push(d);
        last_count = count;
    }
    if last_count != n {
        bail!(
            "ingest {}/{mode} N={n}: sink counted {last_count}, expected {n}",
            shape.key
        );
    }

    let ingest_full_s = secs(median(fulls));
    let ingest_only_s = (ingest_full_s - emit_full_s).max(1e-6);
    let rows_per_s = n as f64 / ingest_only_s;
    let mb_per_s = n as f64 * shape.bytes_per_row / 1e6 / ingest_only_s;
    Ok(IngestCell {
        shape: shape.key,
        mode,
        n,
        ingest_full_s,
        ingest_only_s,
        rows_per_s,
        mb_per_s,
    })
}

fn full_sql(script: &str, n: i64) -> String {
    format!("SELECT {script}({n}, 1) FROM DUAL")
}
fn gen_sql(script: &str, n: i64) -> String {
    format!("SELECT {script}({n}, 0) FROM DUAL")
}

/// Chains the generator's emit output straight into the sink's input, with an
/// explicit derived-table column list at every level (safe regardless of
/// whether an EMITS-declared name would otherwise propagate), and reads the
/// sink's row count back as `TO_CHAR` text.
fn ingest_sql(gen_script: &str, sink_script: &str, col_names: &str, n: i64) -> String {
    let aliased = col_names
        .split(", ")
        .map(|c| format!("g.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT TO_CHAR(s.cnt) FROM (SELECT {sink_script}({aliased}) FROM \
         (SELECT {gen_script}({n}, 1) FROM DUAL) g({col_names})) s(cnt)"
    )
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

/// Time an ingest-chain query and return (elapsed, sink row count).
async fn time_ingest_query(conn: &mut Connection, sql: &str) -> Result<(Duration, i64)> {
    let t = Instant::now();
    let s = query_single_string(conn, sql)
        .await?
        .ok_or_else(|| anyhow!("ingest query returned NULL: {sql}"))?;
    let elapsed = t.elapsed();
    let count: i64 = s
        .parse()
        .with_context(|| format!("parsing ingest count '{s}' from: {sql}"))?;
    Ok((elapsed, count))
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}
fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

async fn create_scripts(conn: &mut Connection, udf_path: &str) -> Result<()> {
    for shape in &SHAPES {
        for mode in MODES {
            conn.execute(&format!(
                "CREATE OR REPLACE RUST SET SCRIPT bench.emit_{key}_{mode}(n BIGINT, do_emit BIGINT) \
                 EMITS ({ddl}) AS\n\
                 %udf_object {udf_path};\n/",
                key = shape.key,
                ddl = shape.columns_ddl,
            ))
            .await
            .with_context(|| format!("create rust script emit_{}_{mode}", shape.key))?;
        }

        conn.execute(&format!(
            "CREATE OR REPLACE RUST SET SCRIPT bench.sink_{key}({ddl}) EMITS (cnt BIGINT) AS\n\
             %udf_object {udf_path};\n/",
            key = shape.key,
            ddl = shape.columns_ddl,
        ))
        .await
        .with_context(|| format!("create rust sink script sink_{}", shape.key))?;
    }

    create_python_scripts(conn).await
}

async fn create_python_scripts(conn: &mut Connection) -> Result<()> {
    // Python3 row script (mixed shape). LABEL is 50 chars to match the Rust side.
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

    // Python3 columnar script (pandas DataFrame emit, mixed shape).
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

    // Python3 row script (wide shape: NUMERIC/DATE/TIMESTAMP + VARCHAR).
    conn.execute(
        r#"CREATE OR REPLACE PYTHON3 SET SCRIPT bench.py_wide_row(n BIGINT, do_emit BIGINT) EMITS (id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, label VARCHAR(100)) AS
import datetime
from decimal import Decimal
LABEL = '0123456789012345678901234567890123456789012345678'
BASE_DATE = datetime.date(2020, 1, 1)
BASE_TS = datetime.datetime(2020, 1, 1)
def run(ctx):
    n = ctx.n
    do_emit = ctx.do_emit
    if do_emit:
        for i in range(n):
            amount = Decimal(i * 137 + 4200) / 100
            event_date = BASE_DATE + datetime.timedelta(days=i % 3650)
            event_ts = BASE_TS + datetime.timedelta(seconds=i, microseconds=(i * 137) % 1000000)
            ctx.emit(i, amount, event_date, event_ts, LABEL)
    else:
        for i in range(n):
            row = (i, Decimal(i * 137 + 4200) / 100, BASE_DATE, BASE_TS, LABEL)
        ctx.emit(0, Decimal('42.00'), BASE_DATE, BASE_TS, LABEL)
/"#,
    )
    .await
    .context("create py_wide_row")?;

    // Python3 columnar script (pandas DataFrame emit, wide shape).
    conn.execute(
        r#"CREATE OR REPLACE PYTHON3 SET SCRIPT bench.py_wide_batch(n BIGINT, do_emit BIGINT) EMITS (id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, label VARCHAR(100)) AS
import pandas as pd
import numpy as np
import datetime
from decimal import Decimal
LABEL = '0123456789012345678901234567890123456789012345678'
BASE_DATE = datetime.date(2020, 1, 1)
BASE_TS = pd.Timestamp('2020-01-01')
CHUNK = 100000
def run(ctx):
    n = ctx.n
    do_emit = ctx.do_emit
    emitted = 0
    while emitted < n:
        ln = min(CHUNK, n - emitted)
        ids = np.arange(emitted, emitted + ln, dtype='int64')
        amounts = [Decimal(int(i) * 137 + 4200) / 100 for i in ids]
        dates = [BASE_DATE + datetime.timedelta(days=int(i) % 3650) for i in ids]
        tss = [BASE_TS + pd.Timedelta(seconds=int(i), microseconds=(int(i) * 137) % 1000000) for i in ids]
        df = pd.DataFrame({'id': ids, 'amount': amounts, 'event_date': dates, 'event_ts': tss, 'label': [LABEL] * ln})
        if do_emit:
            ctx.emit(df)
        emitted += ln
    if not do_emit:
        ctx.emit(pd.DataFrame({'id': [0], 'amount': [Decimal('42.00')], 'event_date': [BASE_DATE], 'event_ts': [BASE_TS], 'label': [LABEL]}))
/"#,
    )
    .await
    .context("create py_wide_batch")?;

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

fn print_report(
    startup_rust: Duration,
    startup_py: Duration,
    pandas: bool,
    cells: &[Cell],
    ingest_cells: &[IngestCell],
) {
    println!("\n=== emit-throughput: Rust SLC vs native Python3 ===");
    println!("shapes: mixed = id BIGINT, label VARCHAR(100), val DOUBLE (~66 B/row)");
    println!(
        "        wide  = id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, \
         label VARCHAR(100) (~106 B/row)"
    );
    println!("metric: data transfer = T_full - T_generation (median of {RUNS} runs)\n");

    println!("SLC startup (cold first call, N=1):");
    println!("  rust    : {:>7.1} ms", startup_rust.as_secs_f64() * 1e3);
    println!("  python3 : {:>7.1} ms", startup_py.as_secs_f64() * 1e3);
    if !pandas {
        println!("  (python3 columnar = N/A: pandas absent)");
    }

    println!(
        "\n{:<7} {:<6} {:<8} {:>10} {:>9} {:>9} {:>9} {:>13} {:>10}",
        "shape", "mode", "lang", "N", "gen(s)", "full(s)", "xfer(s)", "rows/s", "MB/s"
    );
    println!("{}", "-".repeat(90));
    for c in cells {
        if c.na {
            println!(
                "{:<7} {:<6} {:<8} {:>10} {:>9} {:>9} {:>9} {:>13} {:>10}",
                c.shape, c.mode, c.lang, c.n, "-", "-", "-", "N/A", "N/A"
            );
            continue;
        }
        println!(
            "{:<7} {:<6} {:<8} {:>10} {:>9.3} {:>9.3} {:>9.3} {:>13.0} {:>10.1}",
            c.shape, c.mode, c.lang, c.n, c.gen_s, c.full_s, c.xfer_s, c.rows_per_s, c.mb_per_s
        );
    }

    // Rust-vs-Python transfer ratio per (shape, mode, N).
    println!("\ntransfer throughput ratio (rust rows/s ÷ python3 rows/s):");
    for shape in &SHAPES {
        for mode in MODES {
            for &n in &NS {
                let r = cells.iter().find(|c| {
                    c.shape == shape.key && c.mode == mode && c.lang == "rust" && c.n == n && !c.na
                });
                let p = cells.iter().find(|c| {
                    c.shape == shape.key
                        && c.mode == mode
                        && c.lang == "python3"
                        && c.n == n
                        && !c.na
                });
                match (r, p) {
                    (Some(r), Some(p)) if p.rows_per_s > 0.0 => println!(
                        "  {:<7} {mode:<6} N={n:<9} {:.2}x",
                        shape.key,
                        r.rows_per_s / p.rows_per_s
                    ),
                    _ => println!("  {:<7} {mode:<6} N={n:<9} N/A", shape.key),
                }
            }
        }
    }

    println!(
        "\ningest (decode-side, Rust only): T_ingest = T_ingest_full - T_emit_full (median of {RUNS} runs)"
    );
    println!(
        "{:<7} {:<6} {:>10} {:>13} {:>13} {:>13} {:>10}",
        "shape", "mode", "N", "ing_full(s)", "ingest(s)", "rows/s", "MB/s"
    );
    println!("{}", "-".repeat(78));
    for c in ingest_cells {
        println!(
            "{:<7} {:<6} {:>10} {:>13.3} {:>13.3} {:>13.0} {:>10.1}",
            c.shape, c.mode, c.n, c.ingest_full_s, c.ingest_only_s, c.rows_per_s, c.mb_per_s
        );
    }
}
