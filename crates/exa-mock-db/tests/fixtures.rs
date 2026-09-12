use std::path::PathBuf;

use exa_mock_db::payload::{encode, int64_col, metadata};
use exa_mock_db::{EmitCollector, EmitCounters, FrameCursor, Int64Columns, Session};
use exa_proto::IterType;
use exa_udf_runtime::Runtime;

fn fixture(lib: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let so = exe.parent().unwrap().join(format!(
        "{}{lib}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(so.exists(), "fixture not found: {so:?}");
    so
}

fn start(
    tag: &str,
    lib: &str,
    script: &str,
    input_iter: IterType,
) -> (
    Session,
    std::thread::JoinHandle<Result<(), exa_udf_runtime::RuntimeError>>,
) {
    let mut session = Session::bind(tag).unwrap();
    let ep = session.endpoint().to_string();
    let client = std::thread::spawn(move || Runtime::new(ep, "mock-test".into()).run(|_| {}));
    let meta = metadata(
        input_iter,
        IterType::PbExactlyOnce,
        vec![int64_col("x")],
        vec![int64_col("y")],
    );
    session.handshake(&fixture(lib), script, meta).unwrap();
    (session, client)
}

#[test]
fn scalar_double_runs_one_emit_per_cycle() {
    let (mut session, client) = start(
        "scalar",
        "scalar_double",
        "SCALAR_DOUBLE",
        IterType::PbExactlyOnce,
    );
    let src = Int64Columns::new(&["x"], |i| vec![10 + i as i64]);
    let input = encode(&src, 6, 2);
    let mut counters = EmitCounters::default();
    let mut collector = EmitCollector::default();
    for cycle in &input.cycles {
        let d = session
            .run_cycle(&mut FrameCursor::new(cycle), &mut counters)
            .unwrap();
        assert!(d.as_nanos() > 0);
    }
    for cycle in &input.cycles {
        session
            .run_cycle(&mut FrameCursor::new(cycle), &mut collector)
            .unwrap();
    }
    session
        .run_cycle(&mut FrameCursor::new(&[]), &mut counters)
        .unwrap();
    session.finish().unwrap();
    client.join().unwrap().unwrap();
    assert_eq!(counters.messages, 2, "one tail MT_EMIT per non-empty cycle");
    assert_eq!(counters.rows, 6);
    assert_eq!(
        counters.with_row_number, 0,
        "the client does not echo row_number"
    );
    assert_eq!(collector.int64_cells(), vec![20, 22, 24, 26, 28, 30]);
}

#[test]
fn set_sum_aggregates_each_cycle_as_a_group() {
    let (mut session, client) = start("set", "set_sum", "SET_SUM", IterType::PbMultiple);
    let src = Int64Columns::new(&["x"], |i| vec![i as i64 + 1]);
    let input = encode(&src, 10, 2);
    let mut collector = EmitCollector::default();
    for cycle in &input.cycles {
        session
            .run_cycle(&mut FrameCursor::new(cycle), &mut collector)
            .unwrap();
    }
    session.finish().unwrap();
    client.join().unwrap().unwrap();
    assert_eq!(collector.rows(), 2);
    assert_eq!(collector.int64_cells(), vec![15, 40]);
}
