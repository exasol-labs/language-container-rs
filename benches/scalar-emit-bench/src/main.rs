//! SCALAR EMITS throughput benchmark: v0.24.0 SLC vs current SLC.
//!
//! Boots Exasol (Docker via `it::Harness`, or external mode), registers both
//! the v0.24.0 and current SLC as separate language aliases, then measures
//! SCALAR EMITS throughput per shape/version at 2M rows.
//!
//! Three shapes: **mixed** (`id BIGINT, label VARCHAR(100), val DOUBLE`),
//! **wide** (`id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts
//! TIMESTAMP, label VARCHAR(100)`), **native** (`id DECIMAL(18,0), val
//! DOUBLE`). Median of 5 runs, versions interleaved per rep.
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

    let system_langs = query_single_string(
        &mut conn,
        "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'",
    )
    .await?
    .ok_or_else(|| anyhow!("could not read system SCRIPT_LANGUAGES"))?;

    let new_slc_lang = upload_slc(&harness, "rustslc_new", &read_current_slc_tarball()?).await?;
    let old_slc_lang = upload_slc(&harness, "rustslc_old", &read_old_slc_tarball()?).await?;

    let combined = format!("{system_langs} RUST_NEW={new_slc_lang} RUST_OLD={old_slc_lang}");
    conn.execute(&format!("ALTER SESSION SET SCRIPT_LANGUAGES='{combined}'"))
        .await
        .context("setting combined SCRIPT_LANGUAGES")?;

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

    // Create source table for table-driven SCALAR benchmarks.
    create_src_table(&mut conn, N).await?;

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

    // ── Dual-emit benchmarks (1 input row from DUAL → N output rows) ────────
    // Versions interleaved per rep to cancel DB-state drift.
    let mut new_cells: Vec<DualCell> = Vec::new();
    let mut old_cells: Vec<DualCell> = Vec::new();
    for shape in &SHAPES {
        let new_script = script_name("bench_new", shape);
        let old_script = script_name("bench_old", shape);

        // Warmup both versions.
        let _ = time_query(&mut conn, &full_sql(&new_script, WARMUP_N)).await?;
        let _ = time_query(&mut conn, &full_sql(&old_script, WARMUP_N)).await?;

        let mut new_fulls = Vec::with_capacity(RUNS);
        let mut old_fulls = Vec::with_capacity(RUNS);

        // Interleaved full runs.
        for _ in 0..RUNS {
            let (d, rows) = time_query(&mut conn, &full_sql(&new_script, N)).await?;
            if rows as i64 != N {
                bail!("bench_new/{}: emitted {rows} rows, expected {N}", shape.key);
            }
            new_fulls.push(d);

            let (d, rows) = time_query(&mut conn, &full_sql(&old_script, N)).await?;
            if rows as i64 != N {
                bail!("bench_old/{}: emitted {rows} rows, expected {N}", shape.key);
            }
            old_fulls.push(d);
        }

        // Interleaved gen runs.
        let mut new_gens = Vec::with_capacity(RUNS);
        let mut old_gens = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let (d, _) = time_query(&mut conn, &gen_sql(&new_script, N)).await?;
            new_gens.push(d);
            let (d, _) = time_query(&mut conn, &gen_sql(&old_script, N)).await?;
            old_gens.push(d);
        }

        new_cells.push(make_dual_cell(shape, new_fulls, new_gens));
        old_cells.push(make_dual_cell(shape, old_fulls, old_gens));
    }

    // ── Table-driven SCALAR (native only: 2M input rows × 1 emit each) ─────
    let native_shape = &SHAPES[2]; // native
    let (table_new, table_old) = run_table_interleaved(&mut conn, native_shape, false).await?;
    // Passthrough (`SELECT k, udf(…)`) requires row_number in MT_EMIT (v0.25.0
    // only). v0.24.0 does not stamp row_number → engine error. Run current only.
    let passthru_new = run_table_single(&mut conn, "bench_new", native_shape, true).await?;

    print_report(
        startup_new,
        startup_old,
        &new_cells,
        &old_cells,
        &table_new,
        &table_old,
        &passthru_new,
    );
    Ok(())
}

fn script_name(schema: &str, shape: &Shape) -> String {
    format!("{schema}.scalar_emit_{}", shape.key)
}

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

async fn create_src_table(conn: &mut Connection, n: i64) -> Result<()> {
    conn.execute("CREATE SCHEMA IF NOT EXISTS bench_src")
        .await
        .ok();
    conn.execute("CREATE OR REPLACE TABLE bench_src.src_2m (k DECIMAL(18,0), v DOUBLE)")
        .await
        .context("create src_2m")?;
    conn.execute(&format!(
        "INSERT INTO bench_src.src_2m \
         SELECT bench_new.scalar_emit_native({n}, 1) FROM DUAL"
    ))
    .await
    .context("populate src_2m")?;
    let count = query_single_string(
        conn,
        "SELECT CAST(COUNT(*) AS VARCHAR(20)) FROM bench_src.src_2m",
    )
    .await?
    .unwrap_or_default();
    eprintln!("[scalar-emit-bench] src_2m populated: {count} rows");
    Ok(())
}

// ── Dual-emit cell (1 DUAL row → N emit rows) ──────────────────────────────

struct DualCell {
    shape: &'static str,
    raw_fulls: Vec<f64>,
    raw_gens: Vec<f64>,
    gen_s: f64,
    full_s: f64,
    xfer_s: f64,
    rows_per_s: f64,
    mb_per_s: f64,
}

fn make_dual_cell(shape: &Shape, fulls: Vec<Duration>, gens: Vec<Duration>) -> DualCell {
    let full_s = secs(median(&fulls));
    let gen_s = secs(median(&gens));
    let xfer_s = (full_s - gen_s).max(1e-6);
    let rows_per_s = N as f64 / xfer_s;
    let mb_per_s = N as f64 * shape.bytes_per_row / 1e6 / xfer_s;
    DualCell {
        shape: shape.key,
        raw_fulls: fulls.iter().map(|d| secs(*d)).collect(),
        raw_gens: gens.iter().map(|d| secs(*d)).collect(),
        gen_s,
        full_s,
        xfer_s,
        rows_per_s,
        mb_per_s,
    }
}

// ── Table-driven cell (N input rows × 1 emit each) ─────────────────────────

struct TableCell {
    raw_fulls: Vec<f64>,
    full_s: f64,
    rows_per_s: f64,
}

async fn run_table_interleaved(
    conn: &mut Connection,
    shape: &Shape,
    passthrough: bool,
) -> Result<(TableCell, TableCell)> {
    let new_sql = table_sql("bench_new", shape.key, passthrough);
    let old_sql = table_sql("bench_old", shape.key, passthrough);

    // Warmup.
    let _ = time_query(conn, &new_sql).await?;
    let _ = time_query(conn, &old_sql).await?;

    let mut new_fulls = Vec::with_capacity(RUNS);
    let mut old_fulls = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let (d, rows) = time_query(conn, &new_sql).await?;
        if rows as i64 != N {
            bail!("table-driven new: got {rows} rows, expected {N}");
        }
        new_fulls.push(d);
        let (d, rows) = time_query(conn, &old_sql).await?;
        if rows as i64 != N {
            bail!("table-driven old: got {rows} rows, expected {N}");
        }
        old_fulls.push(d);
    }

    let new_cell = make_table_cell(&new_fulls);
    let old_cell = make_table_cell(&old_fulls);
    Ok((new_cell, old_cell))
}

async fn run_table_single(
    conn: &mut Connection,
    schema: &str,
    shape: &Shape,
    passthrough: bool,
) -> Result<TableCell> {
    let sql = table_sql(schema, shape.key, passthrough);
    let _ = time_query(conn, &sql).await?;
    let mut fulls = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let (d, rows) = time_query(conn, &sql).await?;
        if rows as i64 != N {
            bail!("table-driven {schema}: got {rows} rows, expected {N}");
        }
        fulls.push(d);
    }
    Ok(make_table_cell(&fulls))
}

fn table_sql(schema: &str, shape_key: &str, passthrough: bool) -> String {
    if passthrough {
        format!("SELECT k, {schema}.scalar_emit_{shape_key}(1, 1) FROM bench_src.src_2m")
    } else {
        format!("SELECT {schema}.scalar_emit_{shape_key}(1, 1) FROM bench_src.src_2m")
    }
}

fn make_table_cell(fulls: &[Duration]) -> TableCell {
    let full_s = secs(median(fulls));
    let rows_per_s = N as f64 / full_s;
    TableCell {
        raw_fulls: fulls.iter().map(|d| secs(*d)).collect(),
        full_s,
        rows_per_s,
    }
}

// ── Query helpers ───────────────────────────────────────────────────────────

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

fn median(v: &[Duration]) -> Duration {
    let mut sorted: Vec<Duration> = v.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
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

// ── Report ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn print_report(
    startup_new: Duration,
    startup_old: Duration,
    new_cells: &[DualCell],
    old_cells: &[DualCell],
    table_new: &TableCell,
    table_old: &TableCell,
    passthru_new: &TableCell,
) {
    println!("\n=== scalar-emit-throughput: v0.25.0 (current) vs v0.24.0 ===");
    println!("shapes: mixed  = id BIGINT, label VARCHAR(100), val DOUBLE (~65 B/row)");
    println!(
        "        wide   = id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, \
         label VARCHAR(100) (~106 B/row)"
    );
    println!("        native = id DECIMAL(18,0), val DOUBLE (~16 B/row)");
    println!("N = {N}, versions interleaved per rep, median of {RUNS} runs\n");

    println!("SCALAR EMITS cold start (N=1):");
    println!(
        "  current (v0.25.0) : {:>7.1} ms",
        startup_new.as_secs_f64() * 1e3
    );
    println!(
        "  old     (v0.24.0) : {:>7.1} ms",
        startup_old.as_secs_f64() * 1e3
    );

    // ── Raw run times ───────────────────────────────────────────────────
    println!("\n── raw run times (seconds) ──");
    for (new, old) in new_cells.iter().zip(old_cells.iter()) {
        print!("  {:<7} current full:", new.shape);
        for v in &new.raw_fulls {
            print!(" {v:.3}");
        }
        print!("  gen:");
        for v in &new.raw_gens {
            print!(" {v:.3}");
        }
        println!();
        print!("  {:<7} v0.24.0 full:", old.shape);
        for v in &old.raw_fulls {
            print!(" {v:.3}");
        }
        print!("  gen:");
        for v in &old.raw_gens {
            print!(" {v:.3}");
        }
        println!();
    }
    print!("  table   current full:");
    for v in &table_new.raw_fulls {
        print!(" {v:.3}");
    }
    println!();
    print!("  table   v0.24.0 full:");
    for v in &table_old.raw_fulls {
        print!(" {v:.3}");
    }
    println!();
    print!("  passthru current full:");
    for v in &passthru_new.raw_fulls {
        print!(" {v:.3}");
    }
    println!("  (v0.24.0 N/A: no row_number)");

    // ── Dual-emit summary (median) ──────────────────────────────────────
    println!(
        "\n── DUAL-EMIT (1 DUAL row → N output rows, median) ──\n\
         {:<7} {:<9} {:>9} {:>9} {:>9} {:>13} {:>10}",
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

    // ── Table-driven summary (median) ───────────────────────────────────
    println!(
        "\n── TABLE-DRIVEN SCALAR native ({N} input rows × 1 emit each, median) ──\n\
         {:<12} {:<9} {:>9} {:>13}",
        "mode", "version", "full(s)", "rows/s"
    );
    println!("{}", "-".repeat(50));
    println!(
        "{:<12} {:<9} {:>9.3} {:>13.0}",
        "table", "current", table_new.full_s, table_new.rows_per_s
    );
    println!(
        "{:<12} {:<9} {:>9.3} {:>13.0}",
        "table", "v0.24.0", table_old.full_s, table_old.rows_per_s
    );
    println!(
        "{:<12} {:<9} {:>9.3} {:>13.0}",
        "passthrough", "current", passthru_new.full_s, passthru_new.rows_per_s
    );
    println!(
        "{:<12} {:<9} {:>9} {:>13}",
        "passthrough", "v0.24.0", "N/A", "N/A (no row_number)"
    );

    // ── Comparison ──────────────────────────────────────────────────────
    println!("\ntransfer throughput ratio (current ÷ v0.24.0):");
    for (new, old) in new_cells.iter().zip(old_cells.iter()) {
        print_ratio(new.shape, new.rows_per_s, old.rows_per_s);
    }
    print_ratio("table", table_new.rows_per_s, table_old.rows_per_s);
    println!(
        "  {:<12} current only (v0.24.0 lacks row_number)",
        "passthrough"
    );
}

fn print_ratio(label: &str, new_rps: f64, old_rps: f64) {
    if old_rps > 0.0 {
        let ratio = new_rps / old_rps;
        let pct = (ratio - 1.0) * 100.0;
        let sign = if pct >= 0.0 { "+" } else { "" };
        println!("  {:<12} {:.2}x ({sign}{:.1}%)", label, ratio, pct);
    } else {
        println!("  {:<12} N/A", label);
    }
}
