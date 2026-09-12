//! `run`: bring up or attach to a database, install SLC and bench UDF, build the source
//! tables, time every cell, write one JSON file.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use arrow::array::RecordBatch;
use exarrow_rs::adbc::Connection;
use it::{Harness, register_slc};
use sha2::{Digest, Sha256};

use crate::cells::{self, CellSpec, Class, SMALL_ROWS};
use crate::results::{CellResult, Meta, RunFile};
use crate::stats;

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
    pub filter: Option<String>,
    pub keep: bool,
    pub udf_debug: Option<String>,
}

const UDF_LIB: &str = "libbench_udfs.so";

struct Db {
    harness: Harness,
    slc: it::SlcRef,
    conn: Connection,
    udf_debug: Option<String>,
}

impl Db {
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
            let value = scalar(
                conn,
                "SELECT SESSION_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_OUTPUT_ADDRESS'",
            )
            .await?;
            eprintln!("  script output redirected to {value:?}");
        }
        Ok(())
    }

    // A UDF that brings its SQL process down closes the session; the next cell needs a new one.
    async fn reconnect(&mut self) -> Result<()> {
        eprintln!("  connection lost; reconnecting ...");
        let mut conn = self.harness.connect().await?;
        Self::prepare_session(&mut conn, &self.slc, self.udf_debug.as_deref()).await?;
        conn.execute("OPEN SCHEMA bench").await?;
        self.conn = conn;
        Ok(())
    }
}

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

async fn scalar(conn: &mut Connection, sql: &str) -> Result<String> {
    let b = conn.query(sql).await?;
    Ok(first_row(&b)?
        .into_iter()
        .flatten()
        .next()
        .unwrap_or_default())
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

async fn time_cell(db: &mut Db, spec: &CellSpec, profile: Profile) -> Result<CellResult> {
    let mut r = CellResult {
        name: spec.name.clone(),
        shape: spec.shape.to_string(),
        class: spec.class.map(str::to_string),
        mode: spec.mode.map(str::to_string),
        groups: spec.groups,
        rows: spec.rows,
        status: "ok".into(),
        ..Default::default()
    };
    let fail = |r: &mut CellResult, status: &str, error: String| {
        r.status = status.into();
        r.error = Some(error);
        r.raw_s.clear();
    };
    for i in 0..(profile.warmup + profile.reps) {
        let started = Instant::now();
        let batches = match db.conn.query(spec.sql.as_str()).await {
            Ok(b) => b,
            Err(e) if spec.passthrough => {
                db.reconnect().await?;
                fail(
                    &mut r,
                    "incorrect",
                    format!(
                        "server closed the session during the pass-through query \
                         (no row_number echoed in MT_EMIT): {e}"
                    ),
                );
                return Ok(r);
            }
            Err(e) => {
                db.reconnect().await?;
                match db.conn.query(spec.sql.as_str()).await {
                    Ok(_) if i == 0 => continue,
                    Ok(_) => fail(&mut r, "error", format!("{e} (succeeded after reconnect)")),
                    Err(e2) => fail(&mut r, "error", format!("{e}; after reconnect: {e2}")),
                }
                return Ok(r);
            }
        };
        let elapsed = started.elapsed().as_secs_f64();
        let row = match first_row(&batches) {
            Ok(row) => row,
            Err(e) => {
                fail(&mut r, "error", e.to_string());
                return Ok(r);
            }
        };
        if let Some(expected) = spec.expect_count
            && row.first().and_then(parse_int) != Some(expected as i64)
        {
            let got = row
                .first()
                .cloned()
                .flatten()
                .unwrap_or_else(|| "NULL".into());
            fail(
                &mut r,
                "incorrect",
                format!("expected first column {expected}, got {got}"),
            );
        }
        if spec.passthrough {
            let mismatches = row.get(1).and_then(parse_int).unwrap_or(-1);
            r.mismatches = Some(mismatches);
            if mismatches != 0 {
                fail(
                    &mut r,
                    "incorrect",
                    format!("{mismatches} pass-through mismatches"),
                );
            }
        }
        if r.status != "ok" {
            r.result = row;
            return Ok(r);
        }
        if i >= profile.warmup {
            if r.result.is_empty() {
                r.result = row;
            }
            r.raw_s.push(elapsed);
        }
    }
    r.min_s = stats::min(&r.raw_s);
    r.median_s = stats::median(&r.raw_s);
    r.rows_per_s = r.median_s.map(|m| spec.rows as f64 / m);
    r.mb_per_s = spec
        .wire_bytes_per_row
        .and_then(|b| r.rows_per_s.map(|rps| rps * b / 1e6));
    Ok(r)
}

pub async fn run(opts: RunOpts) -> Result<PathBuf> {
    let profile = opts.profile;
    let started_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
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
    let db_version = scalar(
        &mut conn,
        "SELECT PARAM_VALUE FROM EXA_METADATA WHERE PARAM_NAME = 'databaseProductVersion'",
    )
    .await?;

    eprintln!("installing schema BENCH, scripts and source tables ...");
    exec_all(
        &mut conn,
        &[
            "CREATE SCHEMA IF NOT EXISTS bench".into(),
            "OPEN SCHEMA bench".into(),
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
            "  {} ({rows} rows) in {:.1}s",
            class.table(),
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
        eprintln!("  {}", summary_row(&r));
        results.push(r);
    }
    let control_median = |name: &str| {
        results
            .iter()
            .find(|r| r.name == name && r.status == "ok")
            .and_then(|r| r.median_s)
    };
    let ratios: Vec<Option<f64>> = specs
        .iter()
        .zip(&results)
        .map(|(spec, r)| Some(r.median_s? / spec.control.as_deref().and_then(control_median)?))
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
            slc_sha256: format!("{:x}", Sha256::digest(&slc_bytes)),
            bench_udfs_sha256: format!("{:x}", Sha256::digest(&so_bytes)),
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

fn summary_row(c: &CellResult) -> String {
    let f = |v: Option<f64>, scale: f64, prec: usize| {
        v.map(|x| format!("{:.*}", prec, x * scale))
            .unwrap_or_else(|| "-".into())
    };
    format!(
        "{:<40} {:>10} {:>10} {:>12} {:>8} {:>8} {}{}",
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
    )
}

pub fn print_summary(path: &Path) -> Result<()> {
    let file = RunFile::read(path)?;
    let m = &file.meta;
    println!(
        "{}{} db {} profile {} n {} reps {}",
        m.commit,
        if m.dirty { " (dirty)" } else { "" },
        m.db_version,
        m.profile,
        m.n,
        m.reps
    );
    println!(
        "{:<40} {:>10} {:>10} {:>12} {:>8} {:>8} status",
        "cell", "median_ms", "min_ms", "rows_per_s", "MB_per_s", "x_ctrl"
    );
    for c in &file.cells {
        println!("{}", summary_row(c));
    }
    Ok(())
}
