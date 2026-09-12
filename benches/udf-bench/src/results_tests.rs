use super::*;

fn sample() -> RunFile {
    RunFile {
        meta: Meta {
            commit: "3e312a5".into(),
            dirty: true,
            n: 250_000,
            started_at: "2026-09-11T22:30:15Z".into(),
            ..Default::default()
        },
        cells: vec![CellResult {
            name: "control_native".into(),
            raw_s: vec![0.1, 0.2, 0.3],
            status: "ok".into(),
            ..Default::default()
        }],
    }
}

#[test]
fn file_name_and_json_round_trip() {
    let run = sample();
    assert_eq!(run.file_name(), "3e312a5-dirty-20260911-223015.json");
    let dir = std::env::temp_dir().join(format!("udf-bench-{}", std::process::id()));
    let path = dir.join("r.json");
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
