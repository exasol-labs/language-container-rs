//! The Tier 2 matrix: column classes, `CREATE SCRIPT` statements, source
//! tables and one SQL statement per cell. Every cell query returns at most one
//! row, and every aggregate references a UDF output column so the optimizer
//! cannot skip the call.

use bench_schema::{WIDE_BATCH_ROWS, wide_ddl};

/// Column class as in `bench-udfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Native,
    Strblock,
    Varchar,
    /// 24 columns, emit-only: no source table, generator cells only.
    Wide,
}

impl Class {
    /// Classes with a source table (RETURNS, SET and control cells).
    pub const ALL: [Class; 3] = [Class::Native, Class::Strblock, Class::Varchar];
    /// Classes a generator can emit.
    pub const GEN: [Class; 4] = [Class::Native, Class::Strblock, Class::Varchar, Class::Wide];

    pub fn name(self) -> &'static str {
        match self {
            Class::Native => "native",
            Class::Strblock => "strblock",
            Class::Varchar => "varchar",
            Class::Wide => "wide",
        }
    }

    /// Column list with types, as it appears in `CREATE SCRIPT`.
    pub fn columns(self) -> String {
        match self {
            Class::Native => "k DECIMAL(18,0), v DOUBLE".into(),
            Class::Strblock => "k DECIMAL(18,0), amount DECIMAL(18,2), d DATE, ts TIMESTAMP".into(),
            Class::Varchar => "k DECIMAL(18,0), label VARCHAR(100)".into(),
            Class::Wide => wide_ddl(),
        }
    }

    /// Generator modes: `(cell suffix, batch_rows parameter)`. The wide batch
    /// generator takes the rows per record batch as its third parameter.
    pub fn gen_modes(self) -> Vec<(&'static str, Option<u64>)> {
        match self {
            Class::Wide => {
                let mut v = vec![("row", None)];
                v.extend(WIDE_BATCH_ROWS.iter().map(|(n, r)| (*n, Some(*r))));
                v
            }
            _ => MODES.iter().map(|m| (*m, None)).collect(),
        }
    }

    /// Rows a generator cell of this class emits for a profile of `n` rows.
    /// A wide row is about 25 times a native row on the wire, so the wide cells
    /// emit `n / WIDE_GEN_DIVISOR` to keep the run inside its time budget while
    /// still moving more bytes than any other cell.
    pub fn gen_rows(self, n: u64) -> u64 {
        match self {
            Class::Wide => (n / WIDE_GEN_DIVISOR).max(1),
            _ => n,
        }
    }

    /// Wire bytes per emitted row of this class, from Tier 1's `bytes/row`
    /// counter column (`cargo bench ... -- scalar_emits_gen`), for the MB/s
    /// figure of the generator cells. Update when a generator or the encoder
    /// changes; `None` prints no MB/s.
    pub fn wire_bytes_per_row(self) -> Option<f64> {
        match self {
            Class::Native => WIRE_BYTES_PER_ROW[0],
            Class::Strblock => WIRE_BYTES_PER_ROW[1],
            Class::Varchar => WIRE_BYTES_PER_ROW[2],
            Class::Wide => WIRE_BYTES_PER_ROW[3],
        }
    }

    /// Bare column names, for passing a source table's row into a script.
    pub fn args(self) -> &'static str {
        match self {
            Class::Native => "k, v",
            Class::Strblock => "k, amount, d, ts",
            Class::Varchar => "k, label",
            Class::Wide => unreachable!("the wide class has no source table"),
        }
    }

    pub fn table(self) -> String {
        format!("bench.src_{}", self.name())
    }

    /// Rows in this class's source table for a profile of `n` rows. The
    /// `strblock` table is `n / STRBLOCK_INPUT_DIVISOR` (never below the largest
    /// group count, never above `n`): the database feeds DATE and TIMESTAMP
    /// columns into a UDF at a few thousand rows per second, so the seven cells
    /// reading that table would otherwise take 25 of a 26-minute `quick` run
    /// while measuring the engine, not the client.
    pub fn input_rows(self, n: u64) -> u64 {
        match self {
            Class::Strblock => (n / STRBLOCK_INPUT_DIVISOR)
                .max(GROUPS.iter().copied().max().unwrap_or(1))
                .min(n),
            Class::Native | Class::Varchar | Class::Wide => n,
        }
    }
}

pub const MODES: [&str; 2] = ["row", "batch"];
pub const GROUPS: [u64; 2] = [1, 1_000];
/// Divisor for the wide generator cells, see [`Class::gen_rows`].
pub const WIDE_GEN_DIVISOR: u64 = 4;
/// Measured wire bytes per row for native, strblock, varchar, wide; see
/// [`Class::wire_bytes_per_row`].
const WIRE_BYTES_PER_ROW: [Option<f64>; 4] = [Some(12.9), Some(60.6), Some(56.9), Some(472.2)];
/// Divisor for the `strblock` source table, see [`Class::input_rows`].
pub const STRBLOCK_INPUT_DIVISOR: u64 = 100;
/// Rows in `bench.src_small`, the pass-through input.
pub const SMALL_ROWS: u64 = 1_000;

/// One `CREATE OR REPLACE RUST ... SCRIPT` statement per entry point. With
/// `debug` the body also carries `%udf_debug_level debug`, for use together
/// with `SET SESSION SCRIPT OUTPUT ADDRESS`.
pub fn scripts(udf_object: &str, debug: bool) -> Vec<String> {
    let mut out = Vec::new();
    let level = if debug {
        "\n%udf_debug_level debug;"
    } else {
        ""
    };
    let mut push = |kind: &str, name: &str, params: &str, output: &str| {
        out.push(format!(
            "CREATE OR REPLACE RUST {kind} SCRIPT bench.{name}({params}) {output} AS\n\
             %udf_object {udf_object};{level}\n/"
        ));
    };
    let gen_params = "n DECIMAL(18,0), do_emit DECIMAL(18,0)";

    let gen_params_batch_rows = "n DECIMAL(18,0), do_emit DECIMAL(18,0), batch_rows DECIMAL(18,0)";

    push(
        "SCALAR",
        "sr_native",
        &Class::Native.columns(),
        "RETURNS DECIMAL(18,0)",
    );
    push(
        "SCALAR",
        "sr_strblock",
        &Class::Strblock.columns(),
        "RETURNS DECIMAL(18,2)",
    );
    push(
        "SCALAR",
        "sr_varchar",
        &Class::Varchar.columns(),
        "RETURNS DECIMAL(18,0)",
    );
    for class in Class::ALL {
        for mode in MODES {
            let emits = format!("EMITS ({})", class.columns());
            push(
                "SCALAR",
                &format!("gen_{}_{mode}", class.name()),
                gen_params,
                &emits,
            );
            push(
                "SET",
                &format!("setgen_{}_{mode}", class.name()),
                gen_params,
                &emits,
            );
            push(
                "SET",
                &format!("set_emit_{}_{mode}", class.name()),
                &class.columns(),
                &emits,
            );
        }
    }
    let wide_emits = format!("EMITS ({})", Class::Wide.columns());
    push("SCALAR", "gen_wide_row", gen_params, &wide_emits);
    push(
        "SCALAR",
        "gen_wide_batch",
        gen_params_batch_rows,
        &wide_emits,
    );
    push("SET", "setgen_wide_row", gen_params, &wide_emits);
    push(
        "SET",
        "setgen_wide_batch",
        gen_params_batch_rows,
        &wide_emits,
    );
    push(
        "SCALAR",
        "pt_native",
        "k DECIMAL(18,0), n DECIMAL(18,0)",
        "EMITS (k_out DECIMAL(18,0))",
    );
    push(
        "SET",
        "set_sum_native",
        &Class::Native.columns(),
        "RETURNS DOUBLE",
    );
    push(
        "SET",
        "set_sum_strblock",
        &Class::Strblock.columns(),
        "RETURNS DECIMAL(36,2)",
    );
    out
}

/// Entry-point suffix and argument list of a generator call.
fn gen_call(
    mode: &str,
    rows: u64,
    do_emit: u64,
    batch_rows: Option<u64>,
) -> (&'static str, String) {
    match batch_rows {
        Some(b) => ("batch", format!("{rows}, {do_emit}, {b}")),
        None if mode == "row" => ("row", format!("{rows}, {do_emit}")),
        None => ("batch", format!("{rows}, {do_emit}")),
    }
}

/// Select list producing one row of `class` from an integer column `n`.
fn row_expr(class: Class) -> &'static str {
    match class {
        Class::Native => "CAST(n AS DECIMAL(18,0)) AS k, CAST(n * 1.5 AS DOUBLE) AS v",
        Class::Strblock => {
            "CAST(n AS DECIMAL(18,0)) AS k, CAST(n * 1.37 + 42 AS DECIMAL(18,2)) AS amount, \
             ADD_DAYS(DATE '2020-01-01', MOD(n, 3650)) AS d, \
             ADD_SECONDS(TIMESTAMP '2020-01-01 00:00:00', n) AS ts"
        }
        Class::Varchar => {
            "CAST(n AS DECIMAL(18,0)) AS k, CAST(LPAD(TO_CHAR(n), 50, '0') AS VARCHAR(100)) AS label"
        }
        Class::Wide => unreachable!("the wide class has no source table"),
    }
}

/// `CREATE TABLE ... AS SELECT` from a value range, no UDF involved.
pub fn source_table_range(table: &str, class: Class, rows: u64) -> String {
    format!(
        "CREATE TABLE {table} AS SELECT {} FROM (VALUES BETWEEN 1 AND {rows}) AS t(n)",
        row_expr(class)
    )
}

/// Fallback when the database rejects the range form: create the table with
/// the class's columns and fill it through the row generator UDF.
pub fn source_table_fallback(table: &str, class: Class, rows: u64) -> [String; 2] {
    [
        format!("CREATE TABLE {table} ({})", class.columns()),
        format!(
            "INSERT INTO {table} SELECT bench.gen_{}_row({rows}, 1) FROM DUAL",
            class.name()
        ),
    ]
}

/// One benchmark cell.
#[derive(Debug, Clone)]
pub struct CellSpec {
    pub name: String,
    pub shape: &'static str,
    pub class: Option<&'static str>,
    pub mode: Option<&'static str>,
    pub groups: Option<u64>,
    pub sql: String,
    /// Rows the cell moves through the UDF, for rows per second.
    pub rows: u64,
    /// Expected value of the first result column, when it is a row count.
    pub expect_count: Option<u64>,
    /// Name of the control cell this one is reported as a ratio of.
    pub control: Option<String>,
    /// Second result column counts pass-through mismatches; must be zero.
    pub passthrough: bool,
    /// Wire bytes per emitted row, for generator cells with a measured figure.
    pub wire_bytes_per_row: Option<f64>,
}

/// The full matrix for `n` table rows (`scalar_emits_gen` and `set_gen` also
/// emit `n`; cells reading the `strblock` table use [`Class::input_rows`]).
pub fn cells(n: u64) -> Vec<CellSpec> {
    let mut v = Vec::new();
    let cell = |name: String, shape: &'static str, sql: String| CellSpec {
        name,
        shape,
        class: None,
        mode: None,
        groups: None,
        sql,
        rows: n,
        expect_count: None,
        control: None,
        passthrough: false,
        wire_bytes_per_row: None,
    };

    for class in Class::ALL {
        let c = class.name();
        v.push(CellSpec {
            class: Some(c),
            rows: class.input_rows(n),
            ..cell(
                format!("control_{c}"),
                "control",
                format!("SELECT SUM(k * 2) FROM {}", class.table()),
            )
        });
    }

    for class in Class::ALL {
        let c = class.name();
        let rows = class.input_rows(n);
        v.push(CellSpec {
            class: Some(c),
            rows,
            expect_count: Some(rows),
            control: Some(format!("control_{c}")),
            ..cell(
                format!("scalar_returns_{c}"),
                "scalar_returns",
                format!(
                    "SELECT COUNT(y), SUM(y) FROM (SELECT bench.sr_{c}({}) AS y FROM {})",
                    class.args(),
                    class.table()
                ),
            )
        });
    }

    for class in Class::GEN {
        let c = class.name();
        let rows = class.gen_rows(n);
        for (mode, batch_rows) in class.gen_modes() {
            // The large wide batch has no generation-only twin: it builds the
            // same rows as the small one.
            let twins: &[(&str, u64, u64)] = if mode == "batch64k" {
                &[("", 1, rows)]
            } else {
                &[("", 1, rows), ("_noemit", 0, 1)]
            };
            for (suffix, do_emit, expect) in twins {
                let (entry, args) = gen_call(mode, rows, *do_emit, batch_rows);
                v.push(CellSpec {
                    class: Some(c),
                    mode: Some(mode),
                    rows,
                    expect_count: Some(*expect),
                    wire_bytes_per_row: if *do_emit == 1 {
                        class.wire_bytes_per_row()
                    } else {
                        None
                    },
                    ..cell(
                        format!("scalar_emits_gen_{c}_{mode}{suffix}"),
                        "scalar_emits_gen",
                        format!(
                            "SELECT COUNT(*), MAX(k) FROM (SELECT bench.gen_{c}_{entry}({args}) FROM DUAL)"
                        ),
                    )
                });
            }
        }
    }

    for class in [Class::Native, Class::Strblock] {
        let c = class.name();
        let rows = class.input_rows(n);
        for g in GROUPS {
            v.push(CellSpec {
                class: Some(c),
                groups: Some(g),
                rows,
                expect_count: Some(g.min(rows)),
                control: Some(format!("control_{c}")),
                ..cell(
                    format!("set_returns_{c}_g{g}"),
                    "set_returns",
                    format!(
                        "SELECT COUNT(s), SUM(s) FROM (SELECT bench.set_sum_{c}({}) AS s FROM {} GROUP BY MOD(k, {g}))",
                        class.args(),
                        class.table()
                    ),
                )
            });
        }
    }

    for class in [Class::Native, Class::Strblock] {
        let c = class.name();
        let rows = class.input_rows(n);
        for mode in MODES {
            for g in GROUPS {
                v.push(CellSpec {
                    class: Some(c),
                    mode: Some(mode),
                    groups: Some(g),
                    rows,
                    expect_count: Some(rows),
                    control: Some(format!("control_{c}")),
                    ..cell(
                        format!("set_emits_{c}_{mode}_g{g}"),
                        "set_emits",
                        format!(
                            "SELECT COUNT(*), MAX(k) FROM (SELECT bench.set_emit_{c}_{mode}({}) FROM {} GROUP BY MOD(k, {g}))",
                            class.args(),
                            class.table()
                        ),
                    )
                });
            }
        }
    }

    for class in Class::GEN {
        let c = class.name();
        let rows = class.gen_rows(n);
        for (mode, batch_rows) in class.gen_modes() {
            if mode == "batch64k" {
                continue;
            }
            let (entry, args) = gen_call(mode, rows, 1, batch_rows);
            v.push(CellSpec {
                class: Some(c),
                mode: Some(mode),
                rows,
                expect_count: Some(rows),
                wire_bytes_per_row: class.wire_bytes_per_row(),
                ..cell(
                    format!("set_gen_{c}_{mode}"),
                    "set_gen",
                    format!(
                        "SELECT COUNT(*), MAX(k) FROM (SELECT bench.setgen_{c}_{entry}({args}) FROM DUAL)"
                    ),
                )
            });
        }
    }
    // Last: on a client that does not echo `row_number`, this query brings the
    // SQL session down, and the reconnect must not disturb any other cell.
    let per_row = (n / SMALL_ROWS).max(1);
    v.push(CellSpec {
        class: Some("native"),
        expect_count: Some(per_row * SMALL_ROWS),
        passthrough: true,
        rows: per_row * SMALL_ROWS,
        ..cell(
            "scalar_emits_pt".into(),
            "scalar_emits_passthrough",
            format!(
                "SELECT COUNT(*), SUM(CASE WHEN k <> k_out THEN 1 ELSE 0 END) \
                 FROM (SELECT k, bench.pt_native(k, {per_row}) FROM bench.src_small)"
            ),
        )
    });

    v
}

#[cfg(test)]
#[path = "cells_tests.rs"]
mod tests;
