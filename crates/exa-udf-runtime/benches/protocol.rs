//! Tier 1: the client side of the wire protocol against `exa-mock-db`, one
//! open session per cell, timed from the `MT_RUN` reply to the client's `MT_DONE`.
//! Knobs: `BENCH_PROFILE` (`quick`|`full`), `BENCH_ROWS`, `BENCH_ROWS_PER_CYCLE`.
//! `-- --test` runs every cell once and asserts row count and emit sizes.

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Duration;

use criterion::measurement::WallTime;
use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, SamplingMode, Throughput, criterion_group,
    criterion_main,
};
use exa_mock_db::payload::{
    self, ColumnClass, EncodedInput, Int64Columns, RowSource, WIDE_BATCH_ROWS,
};
use exa_mock_db::{EmitCounters, FrameCursor, Session};
use exa_proto::exascript_metadata::ColumnDefinition;
use exa_proto::{ExascriptMetadata, IterType};
use exa_udf_runtime::{Runtime, RuntimeError};

/// Calibrated on docker-db 2026.1.1 (2026-09-11): SCALAR input arrives as
/// 2,000-row `MT_NEXT` frames regardless of row width; see benches/README.md.
const DEFAULT_ROWS_PER_CYCLE: u64 = 2_000;
const PASSTHROUGH_INPUT_ROWS: u64 = 1_000;
const GROUPS: [u64; 2] = [1, 1_000];

struct Profile {
    name: &'static str,
    rows: u64,
    warm_up: Duration,
    measurement: Duration,
    sample_size: usize,
}

fn profile() -> Profile {
    let name = std::env::var("BENCH_PROFILE").unwrap_or_else(|_| "quick".into());
    let mut p = match name.as_str() {
        "quick" => Profile {
            name: "quick",
            rows: 250_000,
            warm_up: Duration::from_secs(1),
            measurement: Duration::from_secs(3),
            sample_size: 10,
        },
        "full" => Profile {
            name: "full",
            rows: 1_000_000,
            warm_up: Duration::from_secs(3),
            measurement: Duration::from_secs(5),
            sample_size: 10,
        },
        other => panic!("BENCH_PROFILE must be `quick` or `full`, got {other:?}"),
    };
    if let Ok(rows) = std::env::var("BENCH_ROWS") {
        p.rows = rows
            .parse()
            .unwrap_or_else(|e| panic!("BENCH_ROWS {rows:?}: {e}"));
    }
    p
}

fn rows_per_cycle() -> u64 {
    match std::env::var("BENCH_ROWS_PER_CYCLE") {
        Ok(v) => v
            .parse()
            .unwrap_or_else(|e| panic!("BENCH_ROWS_PER_CYCLE {v:?}: {e}")),
        Err(_) => DEFAULT_ROWS_PER_CYCLE,
    }
}

fn bench_udf() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let so = exe.parent().expect("parent").join(format!(
        "{}bench_udfs{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    assert!(so.exists(), "bench UDF not found: {so:?}");
    so
}

fn cycles_for(rows: u64) -> u64 {
    rows.div_ceil(rows_per_cycle()).max(1)
}

struct Cell {
    session: Session,
    client: Option<JoinHandle<Result<(), RuntimeError>>>,
    input: EncodedInput,
    counters: EmitCounters,
    expected_rows: u64,
    throughput_rows: u64,
}

impl Cell {
    fn open(
        script: &str,
        meta: ExascriptMetadata,
        input: EncodedInput,
        expected_rows: u64,
        throughput_rows: u64,
    ) -> Cell {
        let mut session = Session::bind(&script.to_lowercase()).expect("bind mock session");
        let ep = session.endpoint().to_string();
        let client = std::thread::spawn(move || Runtime::new(ep, "bench".into()).run(|_| {}));
        session
            .handshake(&bench_udf(), script, meta)
            .unwrap_or_else(|e| panic!("{script}: handshake: {e}"));
        Cell {
            session,
            client: Some(client),
            input,
            counters: EmitCounters::default(),
            expected_rows,
            throughput_rows,
        }
    }

    fn run_iteration(&mut self) -> Duration {
        self.input
            .cycles
            .iter()
            .map(|cycle| {
                self.session
                    .run_cycle(&mut FrameCursor::new(cycle), &mut self.counters)
                    .unwrap_or_else(|e| panic!("run cycle: {e}"))
            })
            .sum()
    }

    fn close(mut self) {
        self.session.finish().expect("session teardown");
        self.client
            .take()
            .expect("client thread")
            .join()
            .expect("client thread panicked")
            .expect("client ended with an error");
    }
}

struct Group<'a> {
    name: &'static str,
    inner: BenchmarkGroup<'a, WallTime>,
    report: Vec<(String, EmitCounters, u64)>,
}

impl<'a> Group<'a> {
    fn new(c: &'a mut Criterion, name: &'static str) -> Self {
        let p = profile();
        let mut inner = c.benchmark_group(name);
        inner
            .warm_up_time(p.warm_up)
            .measurement_time(p.measurement)
            .sample_size(p.sample_size)
            .sampling_mode(SamplingMode::Flat);
        Group {
            name,
            inner,
            report: Vec::new(),
        }
    }

    fn bench(&mut self, name: &str, mut cell: Cell) {
        self.inner
            .throughput(Throughput::Elements(cell.throughput_rows));
        let report = &mut self.report;
        self.inner
            .bench_function(BenchmarkId::from_parameter(name), |b| {
                b.iter_custom(|iters| {
                    cell.counters = EmitCounters::default();
                    let total: Duration = (0..iters).map(|_| cell.run_iteration()).sum();
                    assert_eq!(
                        cell.counters.rows,
                        cell.expected_rows * iters,
                        "{name}: emitted rows"
                    );
                    assert_eq!(
                        cell.counters.over_limit, 0,
                        "{name}: MT_EMIT messages over 4,000,000 bytes"
                    );
                    report.retain(|(n, _, _)| n != name);
                    report.push((name.to_string(), cell.counters.clone(), iters));
                    total
                })
            });
        cell.close();
    }

    fn finish(self) -> Vec<(String, EmitCounters, u64)> {
        self.inner.finish();
        if !self.report.is_empty() {
            println!("\n[{}] MT_EMIT per iteration", self.name);
            println!(
                "{:<38} {:>8} {:>10} {:>14} {:>10} {:>12} {:>12} {:>10} {:>10}",
                "cell",
                "msgs",
                "rows",
                "bytes",
                "bytes/row",
                "mean_bytes",
                "max_bytes",
                ">4000000",
                "row_number"
            );
            for (name, c, iters) in &self.report {
                let it = (*iters).max(1) as f64;
                println!(
                    "{:<38} {:>8.1} {:>10.0} {:>14.0} {:>10.1} {:>12.0} {:>12} {:>10.1} {:>10}",
                    name,
                    c.messages as f64 / it,
                    c.rows as f64 / it,
                    c.bytes as f64 / it,
                    c.bytes as f64 / c.rows.max(1) as f64,
                    c.mean_bytes(),
                    c.max_bytes,
                    c.over_limit as f64 / it,
                    if c.with_row_number > 0 { "yes" } else { "no" },
                );
            }
            println!();
        }
        self.report
    }
}

fn returns_col(class: ColumnClass) -> ColumnDefinition {
    match class {
        ColumnClass::Native | ColumnClass::Varchar => payload::int64_col("y"),
        ColumnClass::Strblock => payload::numeric_col("y", 18, 2),
        ColumnClass::Wide => unreachable!(),
    }
}

fn set_sum_col(class: ColumnClass) -> ColumnDefinition {
    match class {
        ColumnClass::Native => payload::double_col("s"),
        ColumnClass::Strblock => payload::numeric_col("s", 36, 2),
        ColumnClass::Varchar | ColumnClass::Wide => unreachable!(),
    }
}

fn gen_modes(class: ColumnClass) -> Vec<(&'static str, Option<u64>)> {
    match class {
        ColumnClass::Wide => std::iter::once(("row", None))
            .chain(WIDE_BATCH_ROWS.iter().map(|(n, r)| (*n, Some(*r))))
            .collect(),
        _ => vec![("row", None), ("batch", None)],
    }
}

fn scalar_returns(c: &mut Criterion) {
    let p = profile();
    let mut g = Group::new(c, "scalar_returns");
    for class in ColumnClass::ALL {
        let meta = payload::metadata(
            IterType::PbExactlyOnce,
            IterType::PbExactlyOnce,
            class.columns(),
            vec![returns_col(class)],
        );
        let input = payload::encode(&class, p.rows, cycles_for(p.rows));
        let script = format!("SR_{}", class.name().to_uppercase());
        g.bench(
            class.name(),
            Cell::open(&script, meta, input, p.rows, p.rows),
        );
    }
    g.finish();
}

fn scalar_emits_gen(c: &mut Criterion) {
    let p = profile();
    let mut g = Group::new(c, "scalar_emits_gen");
    for class in ColumnClass::GEN {
        for (mode, batch_rows) in gen_modes(class) {
            let mut names = vec!["n", "do_emit"];
            let rows = p.rows as i64;
            let fill: Box<dyn Fn(u64) -> Vec<i64>> = match batch_rows {
                Some(b) => {
                    names.push("batch_rows");
                    Box::new(move |_| vec![rows, 1, b as i64])
                }
                None => Box::new(move |_| vec![rows, 1]),
            };
            let params = Int64Columns::new(&names, fill);
            let meta = payload::metadata(
                IterType::PbExactlyOnce,
                IterType::PbMultiple,
                params.columns(),
                class.columns(),
            );
            let input = payload::encode(&params, 1, 1);
            let entry = if mode.starts_with("batch") {
                "BATCH"
            } else {
                "ROW"
            };
            let script = format!("GEN_{}_{entry}", class.name().to_uppercase());
            g.bench(
                &format!("{}_{mode}", class.name()),
                Cell::open(&script, meta, input, p.rows, p.rows),
            );
        }
    }
    g.finish();
}

fn scalar_emits_passthrough(c: &mut Criterion) {
    let p = profile();
    let mut g = Group::new(c, "scalar_emits_passthrough");
    let input_rows = PASSTHROUGH_INPUT_ROWS.min(p.rows).max(1);
    let per_row = (p.rows / input_rows).max(1);
    let meta = payload::metadata(
        IterType::PbExactlyOnce,
        IterType::PbMultiple,
        vec![payload::int64_col("k"), payload::int64_col("n")],
        vec![payload::int64_col("k_out")],
    );
    let src = Int64Columns::new(&["k", "n"], move |i| vec![i as i64, per_row as i64]);
    let input = payload::encode(&src, input_rows, cycles_for(input_rows));
    let expected = input_rows * per_row;
    g.bench(
        "native",
        Cell::open("PT_NATIVE", meta, input, expected, expected),
    );
    let report = g.finish();
    if report.iter().all(|(_, c, _)| c.with_row_number == 0) {
        eprintln!(
            "warning: pass-through MT_EMIT messages carry no row_number; the database \
             cannot place emitted rows beside their input rows"
        );
    }
}

fn set_returns(c: &mut Criterion) {
    let p = profile();
    let mut g = Group::new(c, "set_returns");
    for class in [ColumnClass::Native, ColumnClass::Strblock] {
        for groups in GROUPS {
            let meta = payload::metadata(
                IterType::PbMultiple,
                IterType::PbExactlyOnce,
                class.columns(),
                vec![set_sum_col(class)],
            );
            let input = payload::encode(&class, p.rows, groups);
            let expected = input.cycles.iter().filter(|c| !c.is_empty()).count() as u64;
            let script = format!("SET_SUM_{}", class.name().to_uppercase());
            g.bench(
                &format!("{}_g{groups}", class.name()),
                Cell::open(&script, meta, input, expected, p.rows),
            );
        }
    }
    g.finish();
}

fn set_emits(c: &mut Criterion) {
    let p = profile();
    let mut g = Group::new(c, "set_emits");
    for class in [ColumnClass::Native, ColumnClass::Strblock] {
        for mode in ["row", "batch"] {
            for groups in GROUPS {
                let meta = payload::metadata(
                    IterType::PbMultiple,
                    IterType::PbMultiple,
                    class.columns(),
                    class.columns(),
                );
                let input = payload::encode(&class, p.rows, groups);
                let script = format!(
                    "SET_EMIT_{}_{}",
                    class.name().to_uppercase(),
                    mode.to_uppercase()
                );
                g.bench(
                    &format!("{}_{mode}_g{groups}", class.name()),
                    Cell::open(&script, meta, input, p.rows, p.rows),
                );
            }
        }
    }
    g.finish();
}

fn print_header(_c: &mut Criterion) {
    let p = profile();
    println!(
        "protocol bench: profile={} rows={} rows_per_cycle={} warm_up={:?} measurement={:?} samples={}",
        p.name,
        p.rows,
        rows_per_cycle(),
        p.warm_up,
        p.measurement,
        p.sample_size
    );
}

criterion_group!(
    benches,
    print_header,
    scalar_returns,
    scalar_emits_gen,
    scalar_emits_passthrough,
    set_returns,
    set_emits
);
criterion_main!(benches);
