use super::*;

fn sample() -> RunFile {
    RunFile {
        meta: Meta {
            commit: "3e312a5".into(),
            dirty: true,
            db_version: "2026.1.1".into(),
            db_mode: "docker".into(),
            slc_sha256: "00".into(),
            bench_udfs_sha256: "11".into(),
            profile: "quick".into(),
            n: 250_000,
            reps: 3,
            warmup: 1,
            started_at: "2026-09-11T22:30:15Z".into(),
            source_tables: "range".into(),
        },
        cells: vec![CellResult {
            name: "control_native".into(),
            shape: "control".into(),
            class: Some("native".into()),
            mode: None,
            groups: None,
            rows: 250_000,
            raw_s: vec![0.1, 0.2, 0.3],
            min_s: Some(0.1),
            median_s: Some(0.2),
            rows_per_s: Some(1_250_000.0),
            mb_per_s: None,
            ratio_to_control: None,
            status: "ok".into(),
            error: None,
            mismatches: None,
            result: vec![Some("1".into())],
        }],
    }
}

#[test]
fn file_name_carries_commit_dirty_and_timestamp() {
    assert_eq!(sample().file_name(), "3e312a5-dirty-20260911-223015.json");
}

#[test]
fn json_round_trips() {
    let dir = std::env::temp_dir().join(format!("udf-bench-{}", std::process::id()));
    let path = dir.join("r.json");
    let run = sample();
    run.write(&path).unwrap();
    let back = RunFile::read(&path).unwrap();
    assert_eq!(back.meta.n, 250_000);
    assert_eq!(back.cells[0].raw_s, vec![0.1, 0.2, 0.3]);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("mismatches")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
