use super::*;
use crate::results::Meta;

fn meta(profile: &str, n: u64, commit: &str) -> Meta {
    Meta {
        commit: commit.into(),
        dirty: false,
        db_version: "x".into(),
        db_mode: "external".into(),
        slc_sha256: String::new(),
        bench_udfs_sha256: String::new(),
        profile: profile.into(),
        n,
        reps: 3,
        warmup: 1,
        started_at: "2026-09-11T00:00:00Z".into(),
        source_tables: "range".into(),
    }
}

fn cell(name: &str, status: &str, raw: &[f64]) -> CellResult {
    CellResult {
        name: name.into(),
        shape: "x".into(),
        class: None,
        mode: None,
        groups: None,
        rows: 10,
        raw_s: raw.to_vec(),
        min_s: None,
        median_s: None,
        rows_per_s: None,
        mb_per_s: None,
        ratio_to_control: None,
        status: status.into(),
        error: None,
        mismatches: None,
        result: vec![],
    }
}

fn run(profile: &str, n: u64, commit: &str, cells: Vec<CellResult>) -> RunFile {
    RunFile {
        meta: meta(profile, n, commit),
        cells,
    }
}

#[test]
fn pooling_concatenates_repetitions_across_files() {
    let a = run("full", 10, "a1", vec![cell("c", "ok", &[1.0, 1.1])]);
    let b = run("full", 10, "a2", vec![cell("c", "ok", &[0.9])]);
    let side = pool(&[a, b]).unwrap();
    assert_eq!(side.cells["c"], Some(vec![1.0, 1.1, 0.9]));
    assert_eq!(side.commits, vec!["a1", "a2"]);
}

#[test]
fn a_non_ok_status_poisons_the_cell() {
    let a = run("full", 10, "a", vec![cell("pt", "incorrect", &[])]);
    let b = run("full", 10, "a", vec![cell("pt", "ok", &[1.0])]);
    let side = pool(&[a, b]).unwrap();
    assert_eq!(side.cells["pt"], None);
    assert_eq!(side.statuses["pt"], "incorrect");
}

#[test]
fn mixed_profiles_are_refused() {
    let a = run("full", 10, "a", vec![]);
    let b = run("quick", 10, "a", vec![]);
    assert!(pool(&[a.clone(), b.clone()]).is_err());
    let err = compare(&pool(&[a]).unwrap(), &pool(&[b]).unwrap()).unwrap_err();
    assert!(err.to_string().contains("refusing"));
}

#[test]
fn identical_sides_read_no_change() {
    let raw = [1.00, 1.02, 0.98, 1.01, 0.99];
    let a = run("full", 10, "a", vec![cell("c", "ok", &raw)]);
    let base = pool(&[a.clone(), a.clone()]).unwrap();
    let rows = compare(&base, &base).unwrap();
    assert_eq!(rows[0].verdict, Some(Verdict::NoChange));
    assert!(!rows[0].low_power, "10 pooled samples a side");
    let text = render(&base, &base, &rows);
    assert!(text.contains("no change"));
    assert!(text.contains("band ±8%"));
}

#[test]
fn five_against_five_is_low_power_and_quick_widens_the_band() {
    let base_raw = [1.00, 1.02, 0.98, 1.01, 0.99];
    let change_raw: Vec<f64> = base_raw.iter().map(|x| x * 0.88).collect();
    let b = run("quick", 10, "a", vec![cell("c", "ok", &base_raw)]);
    let c = run("quick", 10, "b", vec![cell("c", "ok", &change_raw)]);
    let rows = compare(&pool(&[b]).unwrap(), &pool(&[c]).unwrap()).unwrap();
    assert!(rows[0].low_power);
    // A 12 percent speedup is inside the 15 percent quick band.
    assert_eq!(rows[0].verdict, Some(Verdict::Small));
}

#[test]
fn a_cell_missing_on_one_side_is_reported_not_compared() {
    let b = run("full", 10, "a", vec![cell("c", "ok", &[1.0, 1.0])]);
    let c = run("full", 10, "b", vec![cell("d", "error", &[])]);
    let rows = compare(&pool(&[b]).unwrap(), &pool(&[c]).unwrap()).unwrap();
    let by = |n: &str| rows.iter().find(|r| r.name == n).unwrap();
    assert_eq!(by("c").note.as_deref(), Some("base ok / change missing"));
    assert_eq!(by("d").note.as_deref(), Some("base missing / change error"));
    assert!(by("c").verdict.is_none());
}

#[test]
fn cells_with_different_row_counts_get_a_note_instead_of_a_verdict() {
    let mut small = cell("c", "ok", &[1.0, 1.1, 1.2]);
    small.rows = 5;
    let base = pool(&[run(
        "quick",
        10,
        "a",
        vec![cell("c", "ok", &[1.0, 1.1, 1.2])],
    )])
    .unwrap();
    let change = pool(&[run("quick", 10, "b", vec![small])]).unwrap();
    let rows = compare(&base, &change).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].verdict.is_none());
    assert_eq!(
        rows[0].note.as_deref(),
        Some("rows differ: base 10 / change 5")
    );
}

#[test]
fn pooling_refuses_mixed_row_counts_on_one_side() {
    let mut small = cell("c", "ok", &[1.0]);
    small.rows = 5;
    let a = run("quick", 10, "a1", vec![cell("c", "ok", &[1.0])]);
    let b = run("quick", 10, "a2", vec![small]);
    let err = pool(&[a, b]).unwrap_err().to_string();
    assert!(err.contains("mixed row counts for c"), "{err}");
}
