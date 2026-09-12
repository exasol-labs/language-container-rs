//! `compare`: pool the raw repetitions of two sets of run files and print a
//! verdict per cell.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, bail};

use crate::results::{CellResult, RunFile};
use crate::stats::{self, Verdict, Welch};

/// Practical band per profile, percent.
pub fn band_pct(profile: &str) -> f64 {
    match profile {
        "full" => 8.0,
        _ => 15.0,
    }
}

/// Fewer pooled samples than this on either side flags the verdict.
pub const LOW_POWER_BELOW: usize = 8;

/// One side of a comparison: raw times pooled per cell.
#[derive(Debug, Default)]
pub struct Side {
    pub profile: String,
    pub n: u64,
    pub commits: Vec<String>,
    /// cell name → pooled repetitions (`None` when any file reports a non-ok status).
    pub cells: BTreeMap<String, Option<Vec<f64>>>,
    pub statuses: BTreeMap<String, String>,
    /// Rows per cell, so a cell whose row count changed between the two
    /// sides (for example the `strblock` table size) is never compared.
    pub rows: BTreeMap<String, u64>,
}

pub fn pool(runs: &[RunFile]) -> Result<Side> {
    let Some(first) = runs.first() else {
        bail!("no run files on one side");
    };
    let mut side = Side {
        profile: first.meta.profile.clone(),
        n: first.meta.n,
        ..Side::default()
    };
    for run in runs {
        if run.meta.profile != side.profile || run.meta.n != side.n {
            bail!(
                "mixed profiles on one side: {}/{} vs {}/{}",
                side.profile,
                side.n,
                run.meta.profile,
                run.meta.n
            );
        }
        side.commits.push(run.meta.commit.clone());
        for cell in &run.cells {
            match side.rows.insert(cell.name.clone(), cell.rows) {
                Some(prev) if prev != cell.rows => bail!(
                    "mixed row counts for {} on one side: {prev} vs {}",
                    cell.name,
                    cell.rows
                ),
                _ => {}
            }
            let entry = side
                .cells
                .entry(cell.name.clone())
                .or_insert_with(|| Some(Vec::new()));
            if cell.status == "ok" {
                if let Some(v) = entry {
                    v.extend(cell.raw_s.iter().copied());
                }
            } else {
                *entry = None;
                side.statuses.insert(cell.name.clone(), cell.status.clone());
            }
        }
    }
    Ok(side)
}

/// One row of the comparison table.
#[derive(Debug)]
pub struct Row {
    pub name: String,
    pub base_median: Option<f64>,
    pub base_min: Option<f64>,
    pub change_median: Option<f64>,
    pub change_min: Option<f64>,
    pub welch: Option<Welch>,
    pub median_delta_pct: Option<f64>,
    pub verdict: Option<Verdict>,
    pub outliers: usize,
    pub low_power: bool,
    pub note: Option<String>,
}

pub fn compare(base: &Side, change: &Side) -> Result<Vec<Row>> {
    if base.profile != change.profile || base.n != change.n {
        bail!(
            "refusing to compare profiles {}/{} (base) and {}/{} (change)",
            base.profile,
            base.n,
            change.profile,
            change.n
        );
    }
    let band = band_pct(&base.profile);
    let mut rows = Vec::new();
    for name in base.cells.keys().chain(change.cells.keys()) {
        if rows.iter().any(|r: &Row| &r.name == name) {
            continue;
        }
        let b = base.cells.get(name).cloned().flatten();
        let c = change.cells.get(name).cloned().flatten();
        let mut row = Row {
            name: name.clone(),
            base_median: b.as_deref().and_then(stats::median),
            base_min: b.as_deref().and_then(stats::min),
            change_median: c.as_deref().and_then(stats::median),
            change_min: c.as_deref().and_then(stats::min),
            welch: None,
            median_delta_pct: None,
            verdict: None,
            outliers: 0,
            low_power: false,
            note: None,
        };
        let (br, cr) = (
            base.rows.get(name).copied().unwrap_or(0),
            change.rows.get(name).copied().unwrap_or(0),
        );
        match (&b, &c) {
            (Some(_), Some(_)) if br != cr => {
                row.note = Some(format!("rows differ: base {br} / change {cr}"));
            }
            (Some(b), Some(c)) => {
                row.outliers = stats::tukey_outliers(b).len() + stats::tukey_outliers(c).len();
                row.low_power = b.len() < LOW_POWER_BELOW || c.len() < LOW_POWER_BELOW;
                if let (Some(bm), Some(cm)) = (row.base_median, row.change_median) {
                    row.median_delta_pct = Some((cm / bm - 1.0) * 100.0);
                }
                row.welch = stats::welch_log(b, c);
                row.verdict = match (&row.welch, row.median_delta_pct) {
                    (Some(w), Some(d)) => Some(stats::verdict(w, d, band)),
                    _ => {
                        row.note = Some("fewer than two samples on a side".into());
                        None
                    }
                };
            }
            _ => {
                let status = |side: &Side| {
                    side.statuses
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| "missing".into())
                };
                row.note = Some(format!(
                    "base {} / change {}",
                    if b.is_some() {
                        "ok".into()
                    } else {
                        status(base)
                    },
                    if c.is_some() {
                        "ok".into()
                    } else {
                        status(change)
                    }
                ));
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

fn ms(v: Option<f64>) -> String {
    v.map(|s| format!("{:.1}", s * 1000.0))
        .unwrap_or_else(|| "-".into())
}

pub fn render(base: &Side, change: &Side, rows: &[Row]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "profile {} n {} band ±{}%  base {} ({} files)  change {} ({} files)\n",
        base.profile,
        base.n,
        band_pct(&base.profile),
        base.commits.join(","),
        base.commits.len(),
        change.commits.join(","),
        change.commits.len()
    ));
    out.push_str(&format!(
        "{:<40} {:>10} {:>10} {:>10} {:>10}  {:<28} {:<10} {}\n",
        "cell", "base_ms", "base_min", "chg_ms", "chg_min", "delta [95% CI]", "verdict", "flags"
    ));
    for r in rows {
        let delta = match (&r.welch, r.median_delta_pct) {
            (Some(w), Some(d)) => {
                format!("{d:+.1}% [{:+.1}%, {:+.1}%]", w.ci_low_pct, w.ci_high_pct)
            }
            _ => "-".into(),
        };
        let mut flags = Vec::new();
        if r.outliers > 0 {
            flags.push(format!("{} outlier(s)", r.outliers));
        }
        if r.low_power {
            flags.push("low power".to_string());
        }
        if let Some(n) = &r.note {
            flags.push(n.clone());
        }
        out.push_str(&format!(
            "{:<40} {:>10} {:>10} {:>10} {:>10}  {:<28} {:<10} {}\n",
            r.name,
            ms(r.base_median),
            ms(r.base_min),
            ms(r.change_median),
            ms(r.change_min),
            delta,
            r.verdict.map(Verdict::label).unwrap_or("-"),
            flags.join("; ")
        ));
    }
    out
}

pub fn run(base: &[std::path::PathBuf], change: &[std::path::PathBuf]) -> Result<String> {
    let read = |paths: &[std::path::PathBuf]| -> Result<Vec<RunFile>> {
        paths.iter().map(|p| RunFile::read(Path::new(p))).collect()
    };
    let base = pool(&read(base)?)?;
    let change = pool(&read(change)?)?;
    let rows = compare(&base, &change)?;
    Ok(render(&base, &change, &rows))
}

/// Cell summary statistics for `run`, shared with `compare` semantics.
pub fn summarize(cell: &mut CellResult) {
    cell.min_s = stats::min(&cell.raw_s);
    cell.median_s = stats::median(&cell.raw_s);
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
