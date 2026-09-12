//! Tier 2 end-to-end UDF shape benchmark driver. See `benches/README.md`.

mod cells;
mod compare;
mod results;
mod run;
mod stats;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "udf-bench",
    about = "End-to-end UDF shape benchmark against an Exasol database"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Time every cell against a database and write one JSON file.
    Run {
        /// `quick` or `full`; defaults to `BENCH_PROFILE`, then `quick`.
        #[arg(long)]
        profile: Option<String>,
        /// Override the profile's row count.
        #[arg(long)]
        rows: Option<u64>,
        /// Output directory for `<commit>-<timestamp>.json`.
        #[arg(long, default_value = "bench-results")]
        out: PathBuf,
        /// Only cells whose name contains this substring.
        #[arg(long)]
        filter: Option<String>,
        /// Keep schema BENCH after the run.
        #[arg(long)]
        keep: bool,
        /// host:port of a listener; scripts get `%udf_debug_level debug` and script output is redirected there.
        #[arg(long)]
        udf_debug: Option<String>,
    },
    /// Pool the repetitions of two sets of run files and print a verdict per cell.
    Compare {
        #[arg(long, required = true, num_args = 1..)]
        base: Vec<PathBuf>,
        #[arg(long, required = true, num_args = 1..)]
        change: Vec<PathBuf>,
    },
    /// Print the cell table of one run file.
    Show { file: PathBuf },
}

fn profile(name: Option<String>) -> Result<run::Profile> {
    let name = name
        .or_else(|| std::env::var("BENCH_PROFILE").ok())
        .unwrap_or_else(|| "quick".into());
    Ok(match name.as_str() {
        "quick" => run::QUICK,
        "full" => run::FULL,
        other => bail!("profile must be `quick` or `full`, got {other:?}"),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Run {
            profile: p,
            rows,
            out,
            filter,
            keep,
            udf_debug,
        } => {
            let mut profile = profile(p)?;
            if let Some(rows) = rows {
                profile.rows = rows;
            }
            let path = run::run(run::RunOpts {
                profile,
                out,
                filter,
                keep,
                udf_debug,
            })
            .await?;
            run::print_summary(&path)?;
        }
        Cmd::Compare { base, change } => print!("{}", compare::run(&base, &change)?),
        Cmd::Show { file } => run::print_summary(&file)?,
    }
    Ok(())
}
