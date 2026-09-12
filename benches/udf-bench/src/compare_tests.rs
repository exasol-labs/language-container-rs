use super::*;
use crate::results::{CellResult, Meta};

fn cell(name: &str, status: &str, raw: &[f64]) -> CellResult {
    CellResult {
        name: name.into(),
        rows: 10,
        raw_s: raw.to_vec(),
        status: status.into(),
        ..Default::default()
    }
}

fn run(profile: &str, n: u64, commit: &str, cells: Vec<CellResult>) -> RunFile {
    RunFile {
        meta: Meta {
            commit: commit.into(),
            profile: profile.into(),
            n,
            ..Default::default()
        },
        cells,
    }
}

#[test]
fn pooling_concatenates_and_a_non_ok_status_poisons() {
    let a = run(
        "full",
        10,
        "a1",
        vec![cell("c", "ok", &[1.0, 1.1]), cell("pt", "incorrect", &[])],
    );
    let b = run(
        "full",
        10,
        "a2",
        vec![cell("c", "ok", &[0.9]), cell("pt", "ok", &[1.0])],
    );
    let side = pool(&[a, b]).unwrap();
    assert_eq!(side.cells["c"], Some(vec![1.0, 1.1, 0.9]));
    assert_eq!(side.commits, vec!["a1", "a2"]);
    assert_eq!(side.cells["pt"], None);
    assert_eq!(side.statuses["pt"], "incorrect");
}

#[test]
fn mixed_profiles_and_row_counts_are_refused() {
    let a = run("full", 10, "a", vec![]);
    let b = run("quick", 10, "a", vec![]);
    assert!(pool(&[a.clone(), b.clone()]).is_err());
    let err = compare(&pool(&[a]).unwrap(), &pool(&[b]).unwrap()).unwrap_err();
    assert!(err.to_string().contains("refusing"));

    let mut small = cell("c", "ok", &[1.0, 1.1, 1.2]);
    small.rows = 5;
    let a = run("quick", 10, "a1", vec![cell("c", "ok", &[1.0, 1.1, 1.2])]);
    let b = run("quick", 10, "a2", vec![small]);
    let err = pool(&[a.clone(), b.clone()]).unwrap_err().to_string();
    assert!(err.contains("mixed row counts for c"), "{err}");
    let rows = compare(&pool(&[a]).unwrap(), &pool(&[b]).unwrap()).unwrap();
    assert!(rows[0].verdict.is_none());
    assert_eq!(
        rows[0].note.as_deref(),
        Some("rows differ: base 10 / change 5")
    );
}

#[test]
fn verdicts_bands_and_low_power() {
    let raw = [1.00, 1.02, 0.98, 1.01, 0.99];
    let a = run("full", 10, "a", vec![cell("c", "ok", &raw)]);
    let base = pool(&[a.clone(), a.clone()]).unwrap();
    let rows = compare(&base, &base).unwrap();
    assert_eq!(rows[0].verdict, Some(Verdict::NoChange));
    assert!(!rows[0].low_power);
    let text = render(&base, &base, &rows);
    assert!(text.contains("no change") && text.contains("band ±8%"));

    let faster: Vec<f64> = raw.iter().map(|x| x * 0.88).collect();
    let b = run("quick", 10, "a", vec![cell("c", "ok", &raw)]);
    let c = run("quick", 10, "b", vec![cell("c", "ok", &faster)]);
    let rows = compare(&pool(&[b]).unwrap(), &pool(&[c]).unwrap()).unwrap();
    assert!(rows[0].low_power);
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
