//! The Tier 2 matrix: classes, scripts, source tables and one single-row query per cell;
//! every aggregate references a UDF output column so the optimizer cannot skip the call.

use bench_schema::{WIDE_BATCH_ROWS, wide_ddl};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Native,
    Strblock,
    Varchar,
    Wide,
}

pub const MODES: [&str; 2] = ["row", "batch"];
pub const GROUPS: [u64; 2] = [1, 1_000];
pub const SMALL_ROWS: u64 = 1_000;
// DATE/TIMESTAMP input reaches a UDF at a few thousand rows/s on docker-db 2026.1.1; at the
// full n the seven strblock input cells took 25 of a 26-minute quick run measuring the engine.
pub const STRBLOCK_INPUT_DIVISOR: u64 = 100;
// A wide row is ~25 native rows on the wire; n / 4 keeps the full run inside its time budget.
pub const WIDE_GEN_DIVISOR: u64 = 4;

impl Class {
    pub const ALL: [Class; 3] = [Class::Native, Class::Strblock, Class::Varchar];
    pub const GEN: [Class; 4] = [Class::Native, Class::Strblock, Class::Varchar, Class::Wide];

    pub fn name(self) -> &'static str {
        match self {
            Class::Native => "native",
            Class::Strblock => "strblock",
            Class::Varchar => "varchar",
            Class::Wide => "wide",
        }
    }

    pub fn columns(self) -> String {
        match self {
            Class::Native => "k DECIMAL(18,0), v DOUBLE".into(),
            Class::Strblock => "k DECIMAL(18,0), amount DECIMAL(18,2), d DATE, ts TIMESTAMP".into(),
            Class::Varchar => "k DECIMAL(18,0), label VARCHAR(100)".into(),
            Class::Wide => wide_ddl(),
        }
    }

    fn args(self) -> &'static str {
        match self {
            Class::Native => "k, v",
            Class::Strblock => "k, amount, d, ts",
            Class::Varchar => "k, label",
            Class::Wide => unreachable!("wide has no source table"),
        }
    }

    pub fn table(self) -> String {
        format!("bench.src_{}", self.name())
    }

    pub fn input_rows(self, n: u64) -> u64 {
        match self {
            Class::Strblock => (n / STRBLOCK_INPUT_DIVISOR).max(GROUPS[1]).min(n),
            _ => n,
        }
    }

    pub fn gen_rows(self, n: u64) -> u64 {
        match self {
            Class::Wide => (n / WIDE_GEN_DIVISOR).max(1),
            _ => n,
        }
    }

    /// `(cell suffix, batch_rows)`; the wide batch generator takes batch_rows as its third parameter.
    fn gen_modes(self) -> Vec<(&'static str, Option<u64>)> {
        match self {
            Class::Wide => std::iter::once(("row", None))
                .chain(WIDE_BATCH_ROWS.iter().map(|&(m, r)| (m, Some(r))))
                .collect(),
            _ => MODES.iter().map(|&m| (m, None)).collect(),
        }
    }

    // Tier 1's `bytes/row` counter column; update together with a generator or the encoder.
    fn wire_bytes_per_row(self) -> Option<f64> {
        Some(match self {
            Class::Native => 13.9,
            Class::Strblock => 61.6,
            Class::Varchar => 57.9,
            Class::Wide => 473.2,
        })
    }
}

pub fn scripts(udf_object: &str, debug: bool) -> Vec<String> {
    let level = if debug {
        "\n%udf_debug_level debug;"
    } else {
        ""
    };
    let script = |kind: &str, name: &str, params: &str, output: &str| {
        format!(
            "CREATE OR REPLACE RUST {kind} SCRIPT bench.{name}({params}) {output} AS\n\
             %udf_object {udf_object};{level}\n/"
        )
    };
    let params = "n DECIMAL(18,0), do_emit DECIMAL(18,0)";
    let params_batch = "n DECIMAL(18,0), do_emit DECIMAL(18,0), batch_rows DECIMAL(18,0)";
    let (native, strblock, varchar) = (
        Class::Native.columns(),
        Class::Strblock.columns(),
        Class::Varchar.columns(),
    );
    let wide = format!("EMITS ({})", Class::Wide.columns());

    let mut out = vec![
        script("SCALAR", "sr_native", &native, "RETURNS DECIMAL(18,0)"),
        script("SCALAR", "sr_strblock", &strblock, "RETURNS DECIMAL(18,2)"),
        script("SCALAR", "sr_varchar", &varchar, "RETURNS DECIMAL(18,0)"),
    ];
    for class in Class::ALL {
        let c = class.name();
        let emits = format!("EMITS ({})", class.columns());
        for mode in MODES {
            out.push(script("SCALAR", &format!("gen_{c}_{mode}"), params, &emits));
            out.push(script("SET", &format!("setgen_{c}_{mode}"), params, &emits));
            out.push(script(
                "SET",
                &format!("set_emit_{c}_{mode}"),
                &class.columns(),
                &emits,
            ));
        }
    }
    out.extend([
        script("SCALAR", "gen_wide_row", params, &wide),
        script("SCALAR", "gen_wide_batch", params_batch, &wide),
        script("SET", "setgen_wide_row", params, &wide),
        script("SET", "setgen_wide_batch", params_batch, &wide),
        script(
            "SCALAR",
            "pt_native",
            "k DECIMAL(18,0), n DECIMAL(18,0)",
            "EMITS (k_out DECIMAL(18,0))",
        ),
        script("SET", "set_sum_native", &native, "RETURNS DOUBLE"),
        script(
            "SET",
            "set_sum_strblock",
            &strblock,
            "RETURNS DECIMAL(36,2)",
        ),
    ]);
    out
}

fn gen_call(
    prefix: &str,
    class: Class,
    mode: &str,
    rows: u64,
    do_emit: u64,
    batch_rows: Option<u64>,
) -> String {
    let entry = if mode == "row" { "row" } else { "batch" };
    let extra = batch_rows.map(|b| format!(", {b}")).unwrap_or_default();
    format!(
        "bench.{prefix}_{}_{entry}({rows}, {do_emit}{extra})",
        class.name()
    )
}

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
        Class::Wide => unreachable!("wide has no source table"),
    }
}

pub fn source_table_range(table: &str, class: Class, rows: u64) -> String {
    format!(
        "CREATE TABLE {table} AS SELECT {} FROM (VALUES BETWEEN 1 AND {rows}) AS t(n)",
        row_expr(class)
    )
}

pub fn source_table_fallback(table: &str, class: Class, rows: u64) -> [String; 2] {
    [
        format!("CREATE TABLE {table} ({})", class.columns()),
        format!(
            "INSERT INTO {table} SELECT bench.gen_{}_row({rows}, 1) FROM DUAL",
            class.name()
        ),
    ]
}

#[derive(Debug, Clone, Default)]
pub struct CellSpec {
    pub name: String,
    pub shape: &'static str,
    pub class: Option<&'static str>,
    pub mode: Option<&'static str>,
    pub groups: Option<u64>,
    pub sql: String,
    pub rows: u64,
    /// Expected first result column, when it is a row count.
    pub expect_count: Option<u64>,
    pub control: Option<String>,
    /// Second result column counts pass-through mismatches; must be zero.
    pub passthrough: bool,
    pub wire_bytes_per_row: Option<f64>,
}

impl CellSpec {
    fn new(name: String, shape: &'static str, class: Class, rows: u64, sql: String) -> Self {
        CellSpec {
            name,
            shape,
            class: Some(class.name()),
            rows,
            sql,
            ..Default::default()
        }
    }

    fn expect(mut self, n: u64) -> Self {
        self.expect_count = Some(n);
        self
    }

    fn mode(mut self, m: &'static str) -> Self {
        self.mode = Some(m);
        self
    }

    fn groups(mut self, g: u64) -> Self {
        self.groups = Some(g);
        self
    }

    fn control(mut self, class: Class) -> Self {
        self.control = Some(format!("control_{}", class.name()));
        self
    }

    fn wire(mut self, bytes: Option<f64>) -> Self {
        self.wire_bytes_per_row = bytes;
        self
    }
}

pub fn cells(n: u64) -> Vec<CellSpec> {
    let mut v = Vec::new();
    for class in Class::ALL {
        let (c, rows, t) = (class.name(), class.input_rows(n), class.table());
        v.push(CellSpec::new(
            format!("control_{c}"),
            "control",
            class,
            rows,
            format!("SELECT SUM(k * 2) FROM {t}"),
        ));
    }
    for class in Class::ALL {
        let (c, rows, t, a) = (
            class.name(),
            class.input_rows(n),
            class.table(),
            class.args(),
        );
        v.push(
            CellSpec::new(
                format!("scalar_returns_{c}"),
                "scalar_returns",
                class,
                rows,
                format!("SELECT COUNT(y), SUM(y) FROM (SELECT bench.sr_{c}({a}) AS y FROM {t})"),
            )
            .expect(rows)
            .control(class),
        );
    }
    for class in Class::GEN {
        let (c, rows) = (class.name(), class.gen_rows(n));
        for (mode, batch_rows) in class.gen_modes() {
            // The large wide batch builds the same rows as the small one: no generation-only twin.
            let twins: &[(&str, u64, u64)] = if mode == "batch64k" {
                &[("", 1, rows)]
            } else {
                &[("", 1, rows), ("_noemit", 0, 1)]
            };
            for &(suffix, do_emit, expect) in twins {
                let call = gen_call("gen", class, mode, rows, do_emit, batch_rows);
                v.push(
                    CellSpec::new(
                        format!("scalar_emits_gen_{c}_{mode}{suffix}"),
                        "scalar_emits_gen",
                        class,
                        rows,
                        format!("SELECT COUNT(*), MAX(k) FROM (SELECT {call} FROM DUAL)"),
                    )
                    .mode(mode)
                    .expect(expect)
                    .wire((do_emit == 1).then(|| class.wire_bytes_per_row()).flatten()),
                );
            }
        }
    }
    for class in [Class::Native, Class::Strblock] {
        let (c, rows, t, a) = (
            class.name(),
            class.input_rows(n),
            class.table(),
            class.args(),
        );
        for g in GROUPS {
            v.push(
                CellSpec::new(
                    format!("set_returns_{c}_g{g}"),
                    "set_returns",
                    class,
                    rows,
                    format!(
                        "SELECT COUNT(s), SUM(s) FROM (SELECT bench.set_sum_{c}({a}) AS s FROM {t} GROUP BY MOD(k, {g}))"
                    ),
                )
                .groups(g)
                .expect(g.min(rows))
                .control(class),
            );
        }
        for mode in MODES {
            for g in GROUPS {
                v.push(
                    CellSpec::new(
                        format!("set_emits_{c}_{mode}_g{g}"),
                        "set_emits",
                        class,
                        rows,
                        format!(
                            "SELECT COUNT(*), MAX(k) FROM (SELECT bench.set_emit_{c}_{mode}({a}) FROM {t} GROUP BY MOD(k, {g}))"
                        ),
                    )
                    .mode(mode)
                    .groups(g)
                    .expect(rows)
                    .control(class),
                );
            }
        }
    }
    for class in Class::GEN {
        let (c, rows) = (class.name(), class.gen_rows(n));
        for (mode, batch_rows) in class.gen_modes() {
            if mode == "batch64k" {
                continue;
            }
            let call = gen_call("setgen", class, mode, rows, 1, batch_rows);
            v.push(
                CellSpec::new(
                    format!("set_gen_{c}_{mode}"),
                    "set_gen",
                    class,
                    rows,
                    format!("SELECT COUNT(*), MAX(k) FROM (SELECT {call} FROM DUAL)"),
                )
                .mode(mode)
                .expect(rows)
                .wire(class.wire_bytes_per_row()),
            );
        }
    }
    // Last: a missing row_number echo closes the SQL session on this query; the reconnect must
    // not disturb any other cell.
    let per_row = (n / SMALL_ROWS).max(1);
    let mut pt = CellSpec::new(
        "scalar_emits_pt".into(),
        "scalar_emits_passthrough",
        Class::Native,
        per_row * SMALL_ROWS,
        format!(
            "SELECT COUNT(*), SUM(CASE WHEN k <> k_out THEN 1 ELSE 0 END) \
             FROM (SELECT k, bench.pt_native(k, {per_row}) FROM bench.src_small)"
        ),
    )
    .expect(per_row * SMALL_ROWS);
    pt.passthrough = true;
    v.push(pt);
    v
}

#[cfg(test)]
#[path = "cells_tests.rs"]
mod tests;
