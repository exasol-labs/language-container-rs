//! SCALAR EMITS throughput benchmark: v0.24.0 SLC vs current SLC.
//!
//! Boots Exasol (Docker via `it::Harness`, or external mode), registers both
//! the v0.24.0 and current SLC as separate language aliases, then measures
//! SCALAR EMITS throughput per shape/version at 2M rows.
//!
//! Three shapes: **mixed** (`id BIGINT, label VARCHAR(100), val DOUBLE`),
//! **wide** (`id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts
//! TIMESTAMP, label VARCHAR(100)`), **native** (`id DECIMAL(18,0), val
//! DOUBLE`). Median of 5 runs.
//!
//! Run: see benches/README.md.

use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use exarrow_rs::adbc::Connection;
use it::{BFS_SERVICE, BUCKET, Harness, query_single_string, read_udf_artifact};

const UDF_LIB: &str = "libscalar_emit_bench_udf.so";
const RUNS: usize = 5;
const WARMUP_N: i64 = 100_000;
const N: i64 = 2_000_000;

const GITHUB_REPO: &str = "exasol-labs/language-container-rs";
const OLD_TAG: &str = "v0.24.0";
const OLD_ASSET: &str = "lc-rust-0.24.0.tar.gz";

struct Shape {
    key: &'static str,
    columns_ddl: &'static str,
    bytes_per_row: f64,
}

const SHAPES: [Shape; 3] = [
    Shape {
        key: "mixed",
        columns_ddl: "id BIGINT, label VARCHAR(100), val DOUBLE",
        bytes_per_row: 65.0,
    },
    Shape {
        key: "wide",
        columns_ddl: "id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, \
                       label VARCHAR(100)",
        bytes_per_row: 106.0,
    },
    Shape {
        key: "native",
        columns_ddl: "id DECIMAL(18,0), val DOUBLE",
        bytes_per_row: 16.0,
    },
];

#[tokio::main]
async fn main() -> Result<()> {
    let harness = Harness::start().await?;

    wait_ready_exapump(&harness.host, harness.db_port).await;

    let mut conn = harness.connect().await?;

    // Read the system SCRIPT_LANGUAGES to keep builtins alive.
    let system_langs = query_single_string(
        &mut conn,
        "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'",
    )
    .await?
    .ok_or_else(|| anyhow!("could not read system SCRIPT_LANGUAGES"))?;

    // Upload and register both SLC versions.
    let new_slc_lang = upload_slc(&harness, "rustslc_new", &read_current_slc_tarball()?).await?;
    let old_slc_lang = upload_slc(&harness, "rustslc_old", &read_old_slc_tarball()?).await?;

    let combined = format!("{system_langs} RUST_NEW={new_slc_lang} RUST_OLD={old_slc_lang}");
    conn.execute(&format!("ALTER SESSION SET SCRIPT_LANGUAGES='{combined}'"))
        .await
        .context("setting combined SCRIPT_LANGUAGES")?;

    // Upload both UDF .so files and create scripts.
    let new_udf_path = harness
        .upload_udf("scalar_bench_new.so", read_udf_artifact(UDF_LIB)?)
        .await?;
    let old_udf_path = harness
        .upload_udf("scalar_bench_old.so", read_old_udf_so()?)
        .await?;

    conn.execute("CREATE SCHEMA IF NOT EXISTS bench_new")
        .await
        .ok();
    conn.execute("CREATE SCHEMA IF NOT EXISTS bench_old")
        .await
        .ok();
    create_scripts(&mut conn, "bench_new", "RUST_NEW", &new_udf_path).await?;
    create_scripts(&mut conn, "bench_old", "RUST_OLD", &old_udf_path).await?;

    // Cold-start probes (before warmup).
    let (startup_new, _) = time_query(
        &mut conn,
        "SELECT bench_new.scalar_emit_mixed(1, 1) FROM DUAL",
    )
    .await?;
    let (startup_old, _) = time_query(
        &mut conn,
        "SELECT bench_old.scalar_emit_mixed(1, 1) FROM DUAL",
    )
    .await?;

    // Benchmark matrix: shape × version.
    let mut new_results: Vec<Cell> = Vec::new();
    let mut old_results: Vec<Cell> = Vec::new();
    for shape in &SHAPES {
        new_results.push(
            run_cell(
                &mut conn,
                shape,
                "bench_new",
                &script_name("bench_new", shape),
                N,
            )
            .await?,
        );
        old_results.push(
            run_cell(
                &mut conn,
                shape,
                "bench_old",
                &script_name("bench_old", shape),
                N,
            )
            .await?,
        );
    }

    print_report(startup_new, startup_old, &new_results, &old_results);
    Ok(())
}

fn script_name(schema: &str, shape: &Shape) -> String {
    format!("{schema}.scalar_emit_{}", shape.key)
}

/// Upload an SLC tarball into BucketFS and return the `localzmq+protobuf` URL
/// fragment (without the `RUST_*=` prefix).
async fn upload_slc(harness: &Harness, name: &str, tarball: &[u8]) -> Result<String> {
    harness
        .upload_to_bucketfs(&format!("slc/{name}.tar.gz"), tarball.to_vec())
        .await?;
    let dir = format!("{BFS_SERVICE}/{BUCKET}/slc/{name}");
    Ok(format!(
        "localzmq+protobuf:///{dir}?lang=rust#buckets/{dir}/exaudf/exaudfclient"
    ))
}

fn read_current_slc_tarball() -> Result<Vec<u8>> {
    let path = std::env::var("SLC_TARBALL").map_err(|_| {
        anyhow!(
            "SLC_TARBALL is not set; build the current SLC tarball first:\n  \
             docker build --target artifact --output type=local,dest=<dir> .\n  \
             then: export SLC_TARBALL=<dir>/lc-rs.tar.gz"
        )
    })?;
    std::fs::read(&path).with_context(|| format!("reading SLC_TARBALL at {path:?}"))
}

fn read_old_slc_tarball() -> Result<Vec<u8>> {
    if let Ok(path) = std::env::var("SLC_TARBALL_OLD") {
        return std::fs::read(&path)
            .with_context(|| format!("reading SLC_TARBALL_OLD at {path:?}"));
    }

    eprintln!("[scalar-emit-bench] SLC_TARBALL_OLD not set; downloading {OLD_TAG} from GitHub...");
    let tmp_dir = std::env::temp_dir().join("scalar-emit-bench-slc-old");
    std::fs::create_dir_all(&tmp_dir).ok();
    let dest = tmp_dir.join(OLD_ASSET);

    if dest.exists() {
        eprintln!("[scalar-emit-bench] Using cached {}", dest.display());
        return std::fs::read(&dest)
            .with_context(|| format!("reading cached old SLC at {}", dest.display()));
    }

    let status = Command::new("gh")
        .args([
            "release",
            "download",
            OLD_TAG,
            "--repo",
            GITHUB_REPO,
            "--pattern",
            OLD_ASSET,
            "--dir",
            tmp_dir.to_str().unwrap(),
            "--clobber",
        ])
        .status()
        .context("running `gh release download`; is `gh` installed and authenticated?")?;

    if !status.success() {
        bail!(
            "`gh release download` failed (exit {status}). Set SLC_TARBALL_OLD manually, or \
             download from https://github.com/{GITHUB_REPO}/releases/tag/{OLD_TAG}"
        );
    }

    std::fs::read(&dest)
        .with_context(|| format!("reading downloaded old SLC at {}", dest.display()))
}

fn read_old_udf_so() -> Result<Vec<u8>> {
    let path = std::env::var("BENCH_UDF_SO_OLD").map_err(|_| {
        anyhow!(
            "BENCH_UDF_SO_OLD is not set; build the v0.24.0-compatible UDF .so first:\n  \
             ./benches/scalar-emit-bench/build-old-udf.sh\n  \
             then: export BENCH_UDF_SO_OLD=<path shown by the script>"
        )
    })?;
    std::fs::read(&path).with_context(|| format!("reading BENCH_UDF_SO_OLD at {path:?}"))
}

async fn create_scripts(
    conn: &mut Connection,
    schema: &str,
    lang: &str,
    udf_path: &str,
) -> Result<()> {
    for shape in &SHAPES {
        conn.execute(&format!(
            "CREATE OR REPLACE {lang} SCALAR SCRIPT {schema}.scalar_emit_{key}(\
             n BIGINT, do_emit BIGINT) EMITS ({ddl}) AS\n\
             %udf_object {udf_path};\n/",
            key = shape.key,
            ddl = shape.columns_ddl,
        ))
        .await
        .with_context(|| {
            format!(
                "create {lang} scalar script {schema}.scalar_emit_{}",
                shape.key
            )
        })?;
    }
    Ok(())
}

struct Cell {
    shape: &'static str,
    gen_s: f64,
    full_s: f64,
    xfer_s: f64,
    rows_per_s: f64,
    mb_per_s: f64,
}

async fn run_cell(
    conn: &mut Connection,
    shape: &'static Shape,
    schema: &str,
    script: &str,
    n: i64,
) -> Result<Cell> {
    // Warmup (discarded).
    let _ = time_query(conn, &full_sql(script, WARMUP_N)).await?;

    // T_full — emit all N rows.
    let mut fulls = Vec::with_capacity(RUNS);
    let mut rows_seen = 0usize;
    for _ in 0..RUNS {
        let (d, rows) = time_query(conn, &full_sql(script, n)).await?;
        fulls.push(d);
        rows_seen = rows;
    }
    if rows_seen as i64 != n {
        bail!(
            "{schema}/{}: emitted {rows_seen} rows, expected {n}",
            shape.key,
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
    let xfer_s = (full_s - gen_s).max(1e-6);
    let rows_per_s = n as f64 / xfer_s;
    let mb_per_s = n as f64 * shape.bytes_per_row / 1e6 / xfer_s;
    Ok(Cell {
        shape: shape.key,
        gen_s,
        full_s,
        xfer_s,
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
                eprintln!("[scalar-emit-bench] Exasol ready (exapump probe).");
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("[scalar-emit-bench] exapump not found; relying on connect() retry.");
                return;
            }
            _ => {
                eprintln!("[scalar-emit-bench] exapump probe {attempt}/60 not ready; retry in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    eprintln!("[scalar-emit-bench] exapump never reported ready; continuing to connect() anyway.");
}

fn print_report(
    startup_new: Duration,
    startup_old: Duration,
    new_cells: &[Cell],
    old_cells: &[Cell],
) {
    println!("\n=== scalar-emit-throughput: v0.25.0 (current) vs v0.24.0 ===");
    println!("shapes: mixed  = id BIGINT, label VARCHAR(100), val DOUBLE (~65 B/row)");
    println!(
        "        wide   = id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, \
         label VARCHAR(100) (~106 B/row)"
    );
    println!("        native = id DECIMAL(18,0), val DOUBLE (~16 B/row)");
    println!("N = {N}, metric: data transfer = T_full - T_generation (median of {RUNS} runs)\n");

    println!("SCALAR EMITS cold start (N=1):");
    println!(
        "  current (v0.25.0) : {:>7.1} ms",
        startup_new.as_secs_f64() * 1e3
    );
    println!(
        "  old     (v0.24.0) : {:>7.1} ms",
        startup_old.as_secs_f64() * 1e3
    );

    println!(
        "\n{:<7} {:<9} {:>9} {:>9} {:>9} {:>13} {:>10}",
        "shape", "version", "gen(s)", "full(s)", "xfer(s)", "rows/s", "MB/s"
    );
    println!("{}", "-".repeat(75));

    for (new, old) in new_cells.iter().zip(old_cells.iter()) {
        println!(
            "{:<7} {:<9} {:>9.3} {:>9.3} {:>9.3} {:>13.0} {:>10.1}",
            new.shape, "current", new.gen_s, new.full_s, new.xfer_s, new.rows_per_s, new.mb_per_s
        );
        println!(
            "{:<7} {:<9} {:>9.3} {:>9.3} {:>9.3} {:>13.0} {:>10.1}",
            old.shape, "v0.24.0", old.gen_s, old.full_s, old.xfer_s, old.rows_per_s, old.mb_per_s
        );
    }

    println!("\ntransfer throughput ratio (current rows/s ÷ v0.24.0 rows/s):");
    for (new, old) in new_cells.iter().zip(old_cells.iter()) {
        if old.rows_per_s > 0.0 {
            let ratio = new.rows_per_s / old.rows_per_s;
            let pct = (ratio - 1.0) * 100.0;
            let sign = if pct >= 0.0 { "+" } else { "" };
            println!("  {:<7} {:.2}x ({sign}{:.1}%)", new.shape, ratio, pct);
        } else {
            println!("  {:<7} N/A", new.shape);
        }
    }
}
