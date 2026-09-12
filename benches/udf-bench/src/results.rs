//! JSON result files written by `run` and read by `compare` and `show`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunFile {
    pub meta: Meta,
    pub cells: Vec<CellResult>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    pub commit: String,
    pub dirty: bool,
    pub db_version: String,
    pub db_mode: String,
    pub slc_sha256: String,
    pub bench_udfs_sha256: String,
    pub profile: String,
    pub n: u64,
    pub reps: usize,
    pub warmup: usize,
    pub started_at: String,
    pub source_tables: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CellResult {
    pub name: String,
    pub shape: String,
    pub class: Option<String>,
    pub mode: Option<String>,
    pub groups: Option<u64>,
    #[serde(default)]
    pub rows: u64,
    pub raw_s: Vec<f64>,
    pub min_s: Option<f64>,
    pub median_s: Option<f64>,
    pub rows_per_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb_per_s: Option<f64>,
    pub ratio_to_control: Option<f64>,
    /// `ok`, `incorrect` or `error`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mismatches: Option<i64>,
    #[serde(default)]
    pub result: Vec<Option<String>>,
}

impl RunFile {
    pub fn read(path: &Path) -> Result<RunFile> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {path:?}"))
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {path:?}"))
    }

    pub fn file_name(&self) -> String {
        let dirty = if self.meta.dirty { "-dirty" } else { "" };
        let ts: String = self
            .meta
            .started_at
            .replace([':', '-'], "")
            .replace('T', "-")
            .chars()
            .take(15)
            .collect();
        format!("{}{dirty}-{ts}.json", self.meta.commit)
    }
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod tests;
