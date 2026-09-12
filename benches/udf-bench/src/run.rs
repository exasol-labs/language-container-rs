//! `run`: bring up (or attach to) a database, install the SLC and the bench
//! UDF, build the source tables, time every cell and write one JSON file.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use arrow::array::RecordBatch;
use exarrow_rs::adbc::Connection;
use it::{Harness, register_slc};
use sha2::{Digest, Sha256};

use crate::cells::{self, CellSpec, Class, SMALL_ROWS};
use crate::compare::summarize;
use crate::results::{CellResult, Meta, RunFile};

/// Rows, warm-up and timed repetitions per profile.
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    pub name: &'static str,
    pub rows: u64,
    pub warmup: usize,
    pub reps: usize,
}

pub const QUICK: Profile = Profile {
    name: "quick",
    rows: 250_000,
    warmup: 1,
    reps: 3,
};
pub const FULL: Profile = Profile {
    name: "full",
    rows: 1_000_000,
    warmup: 1,
    reps: 5,
};

pub struct RunOpts {
    pub profile: Profile,
    pub out: PathBuf,
    /// Only cells whose name contains this substring.
    pub filter: Option<String>,
    /// Leave schema `BENCH` in place afterwards (external mode).
    pub keep: bool,
    /// `host:port` of a listener for the UDF debug log; scripts get
    /// `%udf_debug_level debug` and the session redirects script output there.
    pub udf_debug: Option<String>,
}

/// The database side of a run: one session, re-opened when the server drops it.
struct Db {
    harness: Harness,
    slc: it::SlcRef,
    conn: Connection,
    udf_debug: Option<String>,
}

impl Db {
    /// Session state a fresh connection needs before it can run a cell.
    async fn prepare_session(
        conn: &mut Connection,
        slc: &it::SlcRef,
        udf_debug: Option<&str>,
    ) -> Result<()> {
        register_slc(conn, slc).await?;
        if let Some(addr) = udf_debug {
            conn.execute(format!(
                "ALTER SESSION SET SCRIPT_OUTPUT_ADDRESS = '{addr}'"
            ))
            .await
            .context("redirecting script output")?;
            let b = conn
                .query(
                    "SELECT SESSION_VALUE FROM EXA_PARAMETERS \
                     WHERE PARAMETER_NAME = 'SCRIPT_OUTPUT_ADDRESS'",
                )
                .await?;
            let value = first_row(&b)?
                .into_iter()
                .flatten()
                .next()
                .unwrap_or_default();
            eprintln!("  script output redirected to {value:?}");
        }
        Ok(())
    }

    /// Open a new session after the server closed the old one (a UDF that
    /// brings its SQL process down does that) and re-open schema `bench`.
    async fn reconnect(&mut self) -> Result<()> {
        eprintln!("  connection lost; reconnecting ...");
        let mut conn = self.harness.connect().await?;
        Self::prepare_session(&mut conn, &self.slc, self.udf_debug.as_deref()).await?;
        conn.execute("OPEN SCHEMA bench").await?;
        self.conn = conn;
        Ok(())
    }

    /// Run a cell query; after a failure, reconnect once and retry.
    async fn query_with_retry(&mut self, sql: &str) -> Result<Vec<RecordBatch>> {
        match self.conn.query(sql).await {
            Ok(b) => Ok(b),
            Err(first) => {
                self.reconnect().await?;
                self.conn
                    .query(sql)
                    .await
                    .map_err(|second| anyhow!("{first}; after reconnect: {second}"))
            }
        }
    }
}

const UDF_LIB: &str = "libbench_udfs.so";

fn git(args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// First row of a result as text, one entry per column (`None` for NULL).
fn first_row(batches: &[RecordBatch]) -> Result<Vec<Option<String>>> {
    let Some(batch) = batches.iter().find(|b| b.num_rows() > 0) else {
        return Ok(Vec::new());
    };
    batch
        .columns()
        .iter()
        .map(|col| {
            if col.is_null(0) {
                Ok(None)
            } else {
                arrow::util::display::array_value_to_string(col, 0)
                    .map(Some)
                    .map_err(|e| anyhow!("{e}"))
            }
        })
        .collect()
}

fn parse_int(cell: &Option<String>) -> Option<i64> {
    let text = cell.as_deref()?;
    text.parse::<i64>()
        .ok()
        .or_else(|| text.parse::<f64>().ok().map(|f| f.round() as i64))
}

async fn exec_all(conn: &mut Connection, stmts: &[String]) -> Result<()> {
    for s in stmts {
        conn.execute(s.as_str())
            .await
            .with_context(|| format!("executing: {s}"))?;
    }
    Ok(())
}

/// Build one source table, range form first, generator UDF as fallback.
/// Returns which form succeeded.
async fn build_table(
    conn: &mut Connection,
    table: &str,
    class: Class,
    rows: u64,
) -> Result<&'static str> {
    let _ = conn.execute(format!("DROP TABLE IF EXISTS {table}")).await;
    match conn
        .execute(cells::source_table_range(table, class, rows))
        .await
    {
        Ok(_) => Ok("range"),
        Err(e) => {
            eprintln!("  range form rejected for {table} ({e}); filling through the generator UDF");
            let _ = conn.execute(format!("DROP TABLE IF EXISTS {table}")).await;
            exec_all(conn, &cells::source_table_fallback(table, class, rows)).await?;
            Ok("udf")
        }
    }
}

/// Time one cell: warm-up, repetitions, correctness checks.
async fn time_cell(db: &mut Db, spec: &CellSpec, profile: Profile) -> Result<CellResult> {
    let mut result = CellResult {
        name: spec.name.clone(),
        shape: spec.shape.to_string(),
        class: spec.class.map(str::to_string),
        mode: spec.mode.map(str::to_string),
        groups: spec.groups,
        rows: spec.rows,
        raw_s: Vec::new(),
        min_s: None,
        median_s: None,
        rows_per_s: None,
        mb_per_s: None,
        ratio_to_control: None,
        status: "ok".into(),
        error: None,
        mismatches: None,
        result: Vec::new(),
    };
    for i in 0..(profile.warmup + profile.reps) {
        let started = Instant::now();
        let batches = match db.conn.query(spec.sql.as_str()).await {
            Ok(b) => b,
            Err(e) => {
                // The server may have dropped the session; make sure the next
                // cell gets a working one, then give this cell one more try.
                if spec.passthrough {
                    // The server closes the session instead of returning a
                    // row when emitted rows cannot be placed beside their
                    // input rows; that is the correctness gate failing.
                    db.reconnect().await?;
                    result.status = "incorrect".into();
                    result.error = Some(format!(
                        "server closed the session during the pass-through query \
                         (the client does not echo row_number): {e}"
                    ));
                    result.raw_s.clear();
                    return Ok(result);
                }
                let retry = db.query_with_retry(&spec.sql).await;
                match retry {
                    Ok(_) if i == 0 => continue,
                    Ok(_) => {
                        result.status = "error".into();
                        result.error = Some(format!("{e} (succeeded after reconnect)"));
                    }
                    Err(e2) => {
                        result.status = "error".into();
                        result.error = Some(e2.to_string());
                    }
                }
                result.raw_s.clear();
                return Ok(result);
            }
        };
        let elapsed = started.elapsed().as_secs_f64();
        let row = match first_row(&batches) {
            Ok(r) => r,
            Err(e) => {
                result.status = "error".into();
                result.error = Some(e.to_string());
                return Ok(result);
            }
        };
        if let Some(expected) = spec.expect_count {
            let got = row.first().and_then(parse_int);
            if got != Some(expected as i64) {
                result.status = "incorrect".into();
                result.error = Some(format!(
                    "expected first column {expected}, got {}",
                    row.first()
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| "NULL".into())
                ));
            }
        }
        if spec.passthrough {
            let mismatches = row.get(1).and_then(parse_int).unwrap_or(-1);
            result.mismatches = Some(mismatches);
            if mismatches != 0 {
                result.status = "incorrect".into();
                result.error = Some(format!("{mismatches} pass-through mismatches"));
            }
        }
        if result.status != "ok" {
            result.raw_s.clear();
            result.result = row;
            return Ok(result);
        }
        if i >= profile.warmup {
            if result.result.is_empty() {
                result.result = row;
            }
            result.raw_s.push(elapsed);
        }
    }
    summarize(&mut result);
    result.rows_per_s = result.median_s.map(|m| spec.rows as f64 / m);
    result.mb_per_s = match (result.rows_per_s, spec.wire_bytes_per_row) {
        (Some(r), Some(b)) => Some(r * b / 1e6),
        _ => None,
    };
    Ok(result)
}

pub async fn run(opts: RunOpts) -> Result<PathBuf> {
    let profile = opts.profile;
    let started_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Fail fast on everything that does not need the database.
    let commit = git(&["rev-parse", "--short", "HEAD"])?;
    let dirty = !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty();
    let slc_path = std::env::var("SLC_TARBALL").context("SLC_TARBALL is not set")?;
    let slc_bytes = std::fs::read(&slc_path).with_context(|| format!("reading {slc_path}"))?;
    let so_bytes = it::read_udf_artifact(UDF_LIB)
        .context("build it first: cargo build --release -p bench-udfs")?;
    let db_mode = if std::env::var_os("EXASOL_HOST").is_some() {
        "external"
    } else {
        "docker"
    };

    eprintln!(
        "udf-bench run: profile={} rows={} warmup={} reps={} commit={commit}{} db={db_mode}",
        profile.name,
        profile.rows,
        profile.warmup,
        profile.reps,
        if dirty { "-dirty" } else { "" }
    );

    let harness = Harness::start()
        .await
        .context("starting the database harness")?;
    let slc = harness.load_slc().await?;
    let mut conn = harness.connect().await?;
    Db::prepare_session(&mut conn, &slc, opts.udf_debug.as_deref()).await?;
    let udf_object = harness.upload_udf(UDF_LIB, so_bytes.clone()).await?;

    let db_version = {
        let b = conn
            .query(
                "SELECT PARAM_VALUE FROM EXA_METADATA WHERE PARAM_NAME = 'databaseProductVersion'",
            )
            .await?;
        first_row(&b)?
            .into_iter()
            .flatten()
            .next()
            .unwrap_or_default()
    };

    eprintln!("installing schema BENCH, scripts and source tables ...");
    exec_all(
        &mut conn,
        &[
            "CREATE SCHEMA IF NOT EXISTS bench".to_string(),
            "OPEN SCHEMA bench".to_string(),
        ],
    )
    .await?;
    exec_all(
        &mut conn,
        &cells::scripts(&udf_object, opts.udf_debug.is_some()),
    )
    .await?;
    let mut forms = Vec::new();
    for class in Class::ALL {
        let t = Instant::now();
        let rows = class.input_rows(profile.rows);
        forms.push(build_table(&mut conn, &class.table(), class, rows).await?);
        eprintln!(
            "  {} ({} rows) in {:.1}s",
            class.table(),
            rows,
            t.elapsed().as_secs_f64()
        );
    }
    forms.push(build_table(&mut conn, "bench.src_small", Class::Native, SMALL_ROWS).await?);
    let source_tables = if forms.iter().all(|f| *f == "range") {
        "range"
    } else {
        "udf"
    };
    let mut db = Db {
        harness,
        slc,
        conn,
        udf_debug: opts.udf_debug.clone(),
    };

    let specs: Vec<CellSpec> = cells::cells(profile.rows)
        .into_iter()
        .filter(|c| opts.filter.as_deref().is_none_or(|f| c.name.contains(f)))
        .collect();
    eprintln!("timing {} cells ...", specs.len());
    let mut results = Vec::with_capacity(specs.len());
    for spec in &specs {
        let r = time_cell(&mut db, spec, profile).await?;
        match r.status.as_str() {
            "ok" => eprintln!(
                "  {:<40} median {:>8.1} ms  min {:>8.1} ms  {:>10.0} rows/s",
                r.name,
                r.median_s.unwrap_or(0.0) * 1000.0,
                r.min_s.unwrap_or(0.0) * 1000.0,
                r.rows_per_s.unwrap_or(0.0)
            ),
            s => eprintln!("  {:<40} {s}: {}", r.name, r.error.as_deref().unwrap_or("")),
        }
        results.push(r);
    }

    let control_median = |name: &str| -> Option<f64> {
        results
            .iter()
            .find(|r| r.name == name && r.status == "ok")
            .and_then(|r| r.median_s)
    };
    let ratios: Vec<Option<f64>> = specs
        .iter()
        .zip(&results)
        .map(|(spec, r)| {
            let ctrl = spec.control.as_deref().and_then(control_median)?;
            Some(r.median_s? / ctrl)
        })
        .collect();
    for (r, ratio) in results.iter_mut().zip(ratios) {
        r.ratio_to_control = ratio;
    }

    if !opts.keep {
        let _ = db.conn.execute("DROP SCHEMA IF EXISTS bench CASCADE").await;
    }

    let file = RunFile {
        meta: Meta {
            commit,
            dirty,
            db_version,
            db_mode: db_mode.into(),
            slc_sha256: sha256_hex(&slc_bytes),
            bench_udfs_sha256: sha256_hex(&so_bytes),
            profile: profile.name.into(),
            n: profile.rows,
            reps: profile.reps,
            warmup: profile.warmup,
            started_at,
            source_tables: source_tables.into(),
        },
        cells: results,
    };
    let path = opts.out.join(file.file_name());
    file.write(&path)?;
    eprintln!("wrote {}", path.display());
    Ok(path)
}

/// Print a run file as the summary table `run` also logs.
pub fn print_summary(path: &Path) -> Result<()> {
    let file = RunFile::read(path)?;
    println!(
        "{} {} db {} profile {} n {} reps {}",
        file.meta.commit,
        if file.meta.dirty { "(dirty)" } else { "" },
        file.meta.db_version,
        file.meta.profile,
        file.meta.n,
        file.meta.reps
    );
    println!(
        "{:<40} {:>10} {:>10} {:>12} {:>7} {:>8} status",
        "cell", "median_ms", "min_ms", "rows_per_s", "MB_per_s", "x_ctrl"
    );
    for c in &file.cells {
        let f = |v: Option<f64>, scale: f64, prec: usize| {
            v.map(|x| format!("{:.*}", prec, x * scale))
                .unwrap_or_else(|| "-".into())
        };
        println!(
            "{:<40} {:>10} {:>10} {:>12} {:>7} {:>8} {}{}",
            c.name,
            f(c.median_s, 1000.0, 1),
            f(c.min_s, 1000.0, 1),
            f(c.rows_per_s, 1.0, 0),
            f(c.mb_per_s, 1.0, 1),
            f(c.ratio_to_control, 1.0, 2),
            c.status,
            c.error
                .as_deref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        );
    }
    Ok(())
}
