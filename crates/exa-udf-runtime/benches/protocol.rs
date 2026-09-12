//! Tier 1 protocol benchmark: the client side of the wire protocol against the
//! mock engine in `exa-mock-db`, no database.
//!
//! Each cell keeps one session open (handshake and `dlopen` happen once,
//! outside the timed window) and times run cycles from the `MT_RUN` reply to
//! the client's `MT_DONE`. Input frames are pre-encoded per cell, so what is
//! measured is the client's receive, decode, dispatch, UDF body, encode and
//! send, plus one libzmq copy per frame on the mock side.
//!
//! Knobs (environment):
//! - `BENCH_PROFILE`: `quick` (default) or `full`; see [`Profile`].
//! - `BENCH_ROWS`: rows per iteration, overriding the profile.
//! - `BENCH_ROWS_PER_CYCLE`: input rows the mock hands a SCALAR script per
//!   `MT_RUN` cycle; see `DEFAULT_ROWS_PER_CYCLE`.
//!
//! Run: `cargo bench -p exa-udf-runtime --features bench --bench protocol`.
//! Smoke: append `-- --test` (each cell runs once and asserts its row count).

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

/// Input rows the database hands a SCALAR script per `MT_RUN` cycle.
///
/// Calibrated on `exasol/docker-db:2026.1.1` (2026-09-11) from the runtime
/// debug log of a Tier 2 `scalar_returns_native` run: four queries over
/// 50,000 rows, split across four UDF processes, arrived as exactly 100
/// `MT_NEXT` frames, one per `MT_RUN` cycle, so 2,000 rows each. Recalibrate
/// with `udf-bench run --udf-debug host:port --filter scalar_returns_native`
/// and override with `BENCH_ROWS_PER_CYCLE`. A wrong size moves absolute
/// numbers, not the base-versus-change delta.
const DEFAULT_ROWS_PER_CYCLE: u64 = 2_000;

/// Input rows for the pass-through cell; each emits `rows / 1000` rows.
const PASSTHROUGH_INPUT_ROWS: u64 = 1_000;

/// Group counts for the SET cells.
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

/// `libbench_udfs.so`, a dependency of this crate, sits beside the bench binary.
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

fn cycles_for(rows: u64, per_cycle: u64) -> u64 {
    rows.div_ceil(per_cycle).max(1)
}

/// One benchmark cell: a live session against one entry point plus the
/// pre-encoded input every iteration replays.
struct Cell {
    session: Session,
    client: Option<JoinHandle<Result<(), RuntimeError>>>,
    input: EncodedInput,
    counters: EmitCounters,
    /// Rows the client must emit per iteration.
    expected_rows: u64,
    /// Rows the iteration processes, for Criterion's throughput figure.
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
            counters: EmitCounters::new(),
            expected_rows,
            throughput_rows,
        }
    }

    /// Replay every cycle of the input once and return the summed window.
    fn run_iteration(&mut self) -> Duration {
        let mut total = Duration::ZERO;
        for cycle in &self.input.cycles {
            total += self
                .session
                .run_cycle(&mut FrameCursor::new(cycle), &mut self.counters)
                .unwrap_or_else(|e| panic!("run cycle: {e}"));
        }
        total
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

/// Per-cell `MT_EMIT` counters, normalised per iteration, printed after each group.
#[derive(Default)]
struct Report {
    rows: Vec<(String, EmitCounters, u64)>,
}

impl Report {
    fn record(&mut self, name: &str, counters: &EmitCounters, iters: u64) {
        self.rows.retain(|(n, _, _)| n != name);
        self.rows.push((name.to_string(), counters.clone(), iters));
    }

    fn print(&self, group: &str) {
        if self.rows.is_empty() {
            return;
        }
        println!("\n[{group}] MT_EMIT per iteration");
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
        for (name, c, iters) in &self.rows {
            let it = (*iters).max(1) as f64;
            let bytes_per_row = if c.rows == 0 {
                0.0
            } else {
                c.bytes as f64 / c.rows as f64
            };
            println!(
                "{:<38} {:>8.1} {:>10.0} {:>14.0} {:>10.1} {:>12.0} {:>12} {:>10.1} {:>10}",
                name,
                c.messages as f64 / it,
                c.rows as f64 / it,
                c.bytes as f64 / it,
                bytes_per_row,
                c.mean_bytes(),
                c.max_bytes,
                c.over_limit as f64 / it,
                if c.with_row_number > 0 { "yes" } else { "no" },
            );
        }
        println!();
    }
}

fn configure(group: &mut BenchmarkGroup<'_, WallTime>, p: &Profile) {
    group
        .warm_up_time(p.warm_up)
        .measurement_time(p.measurement)
        .sample_size(p.sample_size)
        .sampling_mode(SamplingMode::Flat);
}

fn bench_cell(
    group: &mut BenchmarkGroup<'_, WallTime>,
    name: &str,
    mut cell: Cell,
    report: &mut Report,
) {
    group.throughput(Throughput::Elements(cell.throughput_rows));
    group.bench_function(BenchmarkId::from_parameter(name), |b| {
        b.iter_custom(|iters| {
            cell.counters.reset();
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                total += cell.run_iteration();
            }
            assert_eq!(
                cell.counters.rows,
                cell.expected_rows * iters,
                "{name}: emitted rows"
            );
            report.record(name, &cell.counters, iters);
            total
        })
    });
    cell.close();
}

fn upper(class: ColumnClass) -> String {
    class.name().to_uppercase()
}

fn returns_col(class: ColumnClass) -> ColumnDefinition {
    match class {
        ColumnClass::Native | ColumnClass::Varchar => payload::int64_col("y"),
        ColumnClass::Strblock => payload::numeric_col("y", 18, 2),
        ColumnClass::Wide => unreachable!("no SCALAR RETURNS cell for wide"),
    }
}

fn set_sum_col(class: ColumnClass) -> ColumnDefinition {
    match class {
        ColumnClass::Native => payload::double_col("s"),
        ColumnClass::Strblock => payload::numeric_col("s", 36, 2),
        ColumnClass::Varchar | ColumnClass::Wide => {
            unreachable!("no SET RETURNS cell for varchar or wide")
        }
    }
}

/// Generator modes per class: `(cell suffix, rows per record batch)`. The wide
/// class takes its batch size from the script's third parameter.
fn gen_modes(class: ColumnClass) -> Vec<(&'static str, Option<u64>)> {
    match class {
        ColumnClass::Wide => {
            let mut v = vec![("row", None)];
            v.extend(
                WIDE_BATCH_ROWS
                    .iter()
                    .map(|(name, rows)| (*name, Some(*rows))),
            );
            v
        }
        _ => vec![("row", None), ("batch", None)],
    }
}

fn non_empty_cycles(input: &EncodedInput) -> u64 {
    input.cycles.iter().filter(|c| !c.is_empty()).count() as u64
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

fn scalar_returns(c: &mut Criterion) {
    let p = profile();
    let per_cycle = rows_per_cycle();
    let mut report = Report::default();
    let mut group = c.benchmark_group("scalar_returns");
    configure(&mut group, &p);
    for class in ColumnClass::ALL {
        let meta = payload::metadata(
            IterType::PbExactlyOnce,
            IterType::PbExactlyOnce,
            class.columns(),
            vec![returns_col(class)],
        );
        let input = payload::encode(&class, p.rows, cycles_for(p.rows, per_cycle));
        let cell = Cell::open(&format!("SR_{}", upper(class)), meta, input, p.rows, p.rows);
        bench_cell(&mut group, class.name(), cell, &mut report);
    }
    group.finish();
    report.print("scalar_returns");
}

fn scalar_emits_gen(c: &mut Criterion) {
    let p = profile();
    let mut report = Report::default();
    let mut group = c.benchmark_group("scalar_emits_gen");
    configure(&mut group, &p);
    for class in ColumnClass::GEN {
        for (mode, batch_rows) in gen_modes(class) {
            let mut params_cols = vec![payload::int64_col("n"), payload::int64_col("do_emit")];
            if batch_rows.is_some() {
                params_cols.push(payload::int64_col("batch_rows"));
            }
            let meta = payload::metadata(
                IterType::PbExactlyOnce,
                IterType::PbMultiple,
                params_cols,
                class.columns(),
            );
            let rows = p.rows as i64;
            let params = match batch_rows {
                Some(b) => Int64Columns::new(&["n", "do_emit", "batch_rows"], move |_| {
                    vec![rows, 1, b as i64]
                }),
                None => Int64Columns::new(&["n", "do_emit"], move |_| vec![rows, 1]),
            };
            let input = payload::encode(&params, 1, 1);
            let entry = if mode.starts_with("batch") {
                "BATCH"
            } else {
                "ROW"
            };
            let script = format!("GEN_{}_{entry}", upper(class));
            let cell = Cell::open(&script, meta, input, p.rows, p.rows);
            bench_cell(
                &mut group,
                &format!("{}_{mode}", class.name()),
                cell,
                &mut report,
            );
        }
    }
    group.finish();
    report.print("scalar_emits_gen");
}

fn scalar_emits_passthrough(c: &mut Criterion) {
    let p = profile();
    let per_cycle = rows_per_cycle();
    let mut report = Report::default();
    let mut group = c.benchmark_group("scalar_emits_passthrough");
    configure(&mut group, &p);

    let input_rows = PASSTHROUGH_INPUT_ROWS.min(p.rows).max(1);
    let per_row = (p.rows / input_rows).max(1);
    let meta = payload::metadata(
        IterType::PbExactlyOnce,
        IterType::PbMultiple,
        vec![payload::int64_col("k"), payload::int64_col("n")],
        vec![payload::int64_col("k_out")],
    );
    let src = Int64Columns::new(&["k", "n"], move |i| vec![i as i64, per_row as i64]);
    let input = payload::encode(&src, input_rows, cycles_for(input_rows, per_cycle));
    let expected = input_rows * per_row;
    let cell = Cell::open("PT_NATIVE", meta, input, expected, expected);
    bench_cell(&mut group, "native", cell, &mut report);
    group.finish();
    report.print("scalar_emits_passthrough");

    // Becomes an assertion once the client echoes `row_number`; today it
    // cannot, so the cell records the fact and warns.
    if report.rows.iter().all(|(_, c, _)| c.with_row_number == 0) {
        eprintln!(
            "warning: pass-through MT_EMIT messages carry no row_number; the database \
             cannot place emitted rows beside their input rows"
        );
    }
}

fn set_returns(c: &mut Criterion) {
    let p = profile();
    let mut report = Report::default();
    let mut group = c.benchmark_group("set_returns");
    configure(&mut group, &p);
    for class in [ColumnClass::Native, ColumnClass::Strblock] {
        for groups in GROUPS {
            let meta = payload::metadata(
                IterType::PbMultiple,
                IterType::PbExactlyOnce,
                class.columns(),
                vec![set_sum_col(class)],
            );
            let input = payload::encode(&class, p.rows, groups);
            let expected = non_empty_cycles(&input);
            let cell = Cell::open(
                &format!("SET_SUM_{}", upper(class)),
                meta,
                input,
                expected,
                p.rows,
            );
            bench_cell(
                &mut group,
                &format!("{}_g{groups}", class.name()),
                cell,
                &mut report,
            );
        }
    }
    group.finish();
    report.print("set_returns");
}

fn set_emits(c: &mut Criterion) {
    let p = profile();
    let mut report = Report::default();
    let mut group = c.benchmark_group("set_emits");
    configure(&mut group, &p);
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
                let script = format!("SET_EMIT_{}_{}", upper(class), mode.to_uppercase());
                let cell = Cell::open(&script, meta, input, p.rows, p.rows);
                bench_cell(
                    &mut group,
                    &format!("{}_{mode}_g{groups}", class.name()),
                    cell,
                    &mut report,
                );
            }
        }
    }
    group.finish();
    report.print("set_emits");
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
