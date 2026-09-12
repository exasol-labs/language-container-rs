//! Drive the existing `scalar-double` and `set-sum` fixtures through the mock
//! engine, pinning the cycle protocol and the emit accounting.

use std::path::PathBuf;

use exa_mock_db::payload::{encode, int64_col, metadata};
use exa_mock_db::{EmitCollector, EmitCounters, FrameCursor, Int64Columns, Session};
use exa_proto::IterType;
use exa_udf_runtime::Runtime;

/// Fixture cdylibs are dependencies of this crate, so Cargo leaves them beside
/// the test binary (see `exa-udf-runtime/tests/common/mod.rs`).
fn fixture(lib: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let so = exe.parent().unwrap().join(format!(
        "{}{lib}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(so.exists(), "fixture not found: {so:?}");
    so
}

fn spawn_client(
    endpoint: &str,
) -> std::thread::JoinHandle<Result<(), exa_udf_runtime::RuntimeError>> {
    let ep = endpoint.to_string();
    std::thread::spawn(move || Runtime::new(ep, "mock-test".into()).run(|_| {}))
}

#[test]
fn scalar_double_runs_one_emit_per_cycle() {
    let mut session = Session::bind("scalar").unwrap();
    let client = spawn_client(session.endpoint());
    let meta = metadata(
        IterType::PbExactlyOnce,
        IterType::PbExactlyOnce,
        vec![int64_col("x")],
        vec![int64_col("y")],
    );
    session
        .handshake(&fixture("scalar_double"), "SCALAR_DOUBLE", meta)
        .unwrap();

    let src = Int64Columns::new(&["x"], |i| vec![10 + i as i64]);
    let input = encode(&src, 6, 2);
    let mut collector = EmitCollector::new();
    let mut counters = EmitCounters::new();
    for cycle in &input.cycles {
        let d = session
            .run_cycle(&mut FrameCursor::new(cycle), &mut counters)
            .unwrap();
        assert!(d.as_nanos() > 0);
    }
    // Second pass over the same input through a value-collecting sink.
    for cycle in &input.cycles {
        session
            .run_cycle(&mut FrameCursor::new(cycle), &mut collector)
            .unwrap();
    }
    session.finish().unwrap();
    client.join().unwrap().expect("client ended cleanly");

    assert_eq!(counters.messages, 2, "one tail MT_EMIT per cycle");
    assert_eq!(counters.rows, 6);
    assert_eq!(
        counters.with_row_number, 0,
        "the client does not echo row_number yet"
    );
    assert_eq!(collector.int64_cells(), vec![20, 22, 24, 26, 28, 30]);
}

#[test]
fn set_sum_aggregates_each_cycle_as_a_group() {
    let mut session = Session::bind("set").unwrap();
    let client = spawn_client(session.endpoint());
    let meta = metadata(
        IterType::PbMultiple,
        IterType::PbExactlyOnce,
        vec![int64_col("x")],
        vec![int64_col("y")],
    );
    session
        .handshake(&fixture("set_sum"), "SET_SUM", meta)
        .unwrap();

    let src = Int64Columns::new(&["x"], |i| vec![i as i64 + 1]);
    let input = encode(&src, 10, 2);
    let mut collector = EmitCollector::new();
    for cycle in &input.cycles {
        session
            .run_cycle(&mut FrameCursor::new(cycle), &mut collector)
            .unwrap();
    }
    session.finish().unwrap();
    client.join().unwrap().expect("client ended cleanly");

    assert_eq!(collector.rows(), 2, "one RETURNS row per group");
    assert_eq!(collector.int64_cells(), vec![15, 40]);
}

#[test]
fn empty_cycle_yields_no_emit() {
    let mut session = Session::bind("empty").unwrap();
    let client = spawn_client(session.endpoint());
    let meta = metadata(
        IterType::PbExactlyOnce,
        IterType::PbExactlyOnce,
        vec![int64_col("x")],
        vec![int64_col("y")],
    );
    session
        .handshake(&fixture("scalar_double"), "SCALAR_DOUBLE", meta)
        .unwrap();
    let mut counters = EmitCounters::new();
    session
        .run_cycle(&mut FrameCursor::new(&[]), &mut counters)
        .unwrap();
    session.finish().unwrap();
    client.join().unwrap().unwrap();
    assert_eq!(counters.messages, 0);
}
