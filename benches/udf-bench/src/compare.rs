//! `compare`: pool the repetitions of two sets of run files and print a verdict per cell.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::results::RunFile;
use crate::stats::{self, Verdict, Welch};

pub const LOW_POWER_BELOW: usize = 8;

fn band_pct(profile: &str) -> f64 {
    if profile == "full" { 8.0 } else { 15.0 }
}

#[derive(Debug, Default)]
pub struct Side {
    pub profile: String,
    pub n: u64,
    pub commits: Vec<String>,
    /// `None` once any file reports a non-ok status for the cell.
    pub cells: BTreeMap<String, Option<Vec<f64>>>,
    pub statuses: BTreeMap<String, String>,
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
            if let Some(prev) = side.rows.insert(cell.name.clone(), cell.rows)
                && prev != cell.rows
            {
                bail!(
                    "mixed row counts for {} on one side: {prev} vs {}",
                    cell.name,
                    cell.rows
                );
            }
            let entry = side
                .cells
                .entry(cell.name.clone())
                .or_insert_with(|| Some(Vec::new()));
            if cell.status == "ok" {
                if let Some(v) = entry {
                    v.extend(&cell.raw_s);
                }
            } else {
                *entry = None;
                side.statuses.insert(cell.name.clone(), cell.status.clone());
            }
        }
    }
    Ok(side)
}

#[derive(Debug, Default)]
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
    let names: std::collections::BTreeSet<&String> =
        base.cells.keys().chain(change.cells.keys()).collect();
    let status = |side: &Side, name: &str, present: bool| {
        if present {
            "ok".to_string()
        } else {
            side.statuses
                .get(name)
                .cloned()
                .unwrap_or_else(|| "missing".into())
        }
    };
    let mut rows = Vec::new();
    for name in names {
        let b = base.cells.get(name).cloned().flatten();
        let c = change.cells.get(name).cloned().flatten();
        let mut row = Row {
            name: name.clone(),
            base_median: b.as_deref().and_then(stats::median),
            base_min: b.as_deref().and_then(stats::min),
            change_median: c.as_deref().and_then(stats::median),
            change_min: c.as_deref().and_then(stats::min),
            ..Row::default()
        };
        let (br, cr) = (base.rows.get(name), change.rows.get(name));
        match (&b, &c) {
            (Some(_), Some(_)) if br != cr => {
                row.note = Some(format!(
                    "rows differ: base {} / change {}",
                    br.copied().unwrap_or(0),
                    cr.copied().unwrap_or(0)
                ));
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
                row.note = Some(format!(
                    "base {} / change {}",
                    status(base, name, b.is_some()),
                    status(change, name, c.is_some())
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
    let mut out = format!(
        "profile {} n {} band ±{}%  base {} ({} files)  change {} ({} files)\n\
         {:<40} {:>10} {:>10} {:>10} {:>10}  {:<28} {:<10} {}\n",
        base.profile,
        base.n,
        band_pct(&base.profile),
        base.commits.join(","),
        base.commits.len(),
        change.commits.join(","),
        change.commits.len(),
        "cell",
        "base_ms",
        "base_min",
        "chg_ms",
        "chg_min",
        "delta [95% CI]",
        "verdict",
        "flags"
    );
    for r in rows {
        let delta = match (&r.welch, r.median_delta_pct) {
            (Some(w), Some(d)) => {
                format!("{d:+.1}% [{:+.1}%, {:+.1}%]", w.ci_low_pct, w.ci_high_pct)
            }
            _ => "-".into(),
        };
        let flags: Vec<String> = [
            (r.outliers > 0).then(|| format!("{} outlier(s)", r.outliers)),
            r.low_power.then(|| "low power".to_string()),
            r.note.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
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

pub fn run(base: &[PathBuf], change: &[PathBuf]) -> Result<String> {
    let read = |paths: &[PathBuf]| -> Result<Vec<RunFile>> {
        paths.iter().map(|p| RunFile::read(p)).collect()
    };
    let base = pool(&read(base)?)?;
    let change = pool(&read(change)?)?;
    let rows = compare(&base, &change)?;
    Ok(render(&base, &change, &rows))
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
