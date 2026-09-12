use std::collections::HashSet;

use super::*;

#[test]
fn matrix_has_44_uniquely_named_cells() {
    let cells = cells(1_000_000);
    assert_eq!(cells.len(), 44);
    let names: HashSet<&str> = cells.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names.len(), cells.len());
}

#[test]
fn every_control_reference_resolves() {
    let cells = cells(250_000);
    let names: HashSet<&str> = cells.iter().map(|c| c.name.as_str()).collect();
    for c in &cells {
        if let Some(ctrl) = &c.control {
            assert!(names.contains(ctrl.as_str()), "{}: {ctrl}", c.name);
            assert!(c.shape != "control");
        }
    }
    assert_eq!(cells.iter().filter(|c| c.shape == "control").count(), 3);
}

#[test]
fn passthrough_cell_scales_with_n_and_runs_last() {
    let c = cells(250_000);
    let pt = c.iter().find(|c| c.passthrough).unwrap();
    assert_eq!(c.last().unwrap().name, pt.name);
    assert_eq!(pt.rows, 250_000);
    assert!(pt.sql.contains("pt_native(k, 250)"));
    assert_eq!(pt.expect_count, Some(250_000));
}

#[test]
fn noemit_cells_expect_one_sentinel_row() {
    let c = cells(1000);
    let no = c
        .iter()
        .find(|c| c.name == "scalar_emits_gen_native_batch_noemit")
        .unwrap();
    assert!(no.sql.contains("gen_native_batch(1000, 0)"));
    assert_eq!(no.expect_count, Some(1));
    let yes = c
        .iter()
        .find(|c| c.name == "scalar_emits_gen_native_batch")
        .unwrap();
    assert_eq!(yes.expect_count, Some(1000));
}

#[test]
fn every_script_has_an_entry_point_and_a_cell_uses_it() {
    let scripts = scripts("/buckets/bfsdefault/default/udf/libbench_udfs.so", false);
    assert_eq!(scripts.len(), 3 + 3 * 2 * 3 + 4 + 1 + 2);
    let all_sql: String = cells(10).iter().map(|c| c.sql.clone()).collect();
    for s in &scripts {
        assert!(s.ends_with(";\n/"), "{s}");
        let name = s
            .split("SCRIPT bench.")
            .nth(1)
            .and_then(|r| r.split('(').next())
            .unwrap();
        // set_emit_varchar has scripts but no cell; everything else is exercised.
        if !name.starts_with("set_emit_varchar") {
            assert!(
                all_sql.contains(&format!("bench.{name}(")),
                "unused script {name}"
            );
        }
    }
}

#[test]
fn source_table_forms_agree_on_columns() {
    let range = source_table_range("bench.src_strblock", Class::Strblock, 5);
    assert!(range.contains("VALUES BETWEEN 1 AND 5"));
    for col in ["AS k", "AS amount", "AS d", "AS ts"] {
        assert!(range.contains(col), "{range}");
    }
    let [create, insert] = source_table_fallback("bench.src_strblock", Class::Strblock, 5);
    assert!(create.contains(Class::Strblock.columns().as_str()));
    assert!(insert.contains("gen_strblock_row(5, 1)"));
}

#[test]
fn debug_scripts_carry_the_level_directive() {
    let plain = scripts("/b/x.so", false);
    let debug = scripts("/b/x.so", true);
    assert!(plain.iter().all(|s| !s.contains("%udf_debug_level")));
    assert!(
        debug
            .iter()
            .all(|s| s.contains("%udf_object /b/x.so;\n%udf_debug_level debug;\n/"))
    );
}

#[test]
fn strblock_input_cells_read_a_smaller_table() {
    assert_eq!(Class::Strblock.input_rows(250_000), 2_500);
    assert_eq!(Class::Strblock.input_rows(1_000_000), 10_000);
    assert_eq!(
        Class::Strblock.input_rows(10_000),
        1_000,
        "floor at the largest group count"
    );
    assert_eq!(Class::Strblock.input_rows(500), 500, "never above n");
    assert_eq!(Class::Native.input_rows(250_000), 250_000);
    assert_eq!(Class::Varchar.input_rows(250_000), 250_000);

    let c = cells(250_000);
    let by = |name: &str| c.iter().find(|x| x.name == name).unwrap();
    for name in [
        "control_strblock",
        "scalar_returns_strblock",
        "set_returns_strblock_g1",
        "set_emits_strblock_batch_g1000",
    ] {
        assert_eq!(by(name).rows, 2_500, "{name}");
    }
    assert_eq!(by("scalar_returns_strblock").expect_count, Some(2_500));
    assert_eq!(by("set_returns_strblock_g1000").expect_count, Some(1_000));
    assert_eq!(by("set_emits_strblock_row_g1").expect_count, Some(2_500));
    // Generated strblock rows do not touch the table and keep the full n.
    assert_eq!(by("scalar_emits_gen_strblock_row").rows, 250_000);
    assert_eq!(by("set_gen_strblock_batch").expect_count, Some(250_000));
    assert_eq!(by("scalar_returns_native").rows, 250_000);
}

#[test]
fn wide_cells_are_generator_only_with_a_batch_rows_parameter() {
    let c = cells(250_000);
    let by = |name: &str| c.iter().find(|x| x.name == name).unwrap();
    let wide: Vec<&CellSpec> = c.iter().filter(|x| x.class == Some("wide")).collect();
    assert_eq!(
        wide.len(),
        7,
        "{:?}",
        wide.iter().map(|x| &x.name).collect::<Vec<_>>()
    );
    assert!(
        wide.iter()
            .all(|x| x.shape == "scalar_emits_gen" || x.shape == "set_gen")
    );
    assert!(wide.iter().all(|x| x.rows == 250_000 / WIDE_GEN_DIVISOR));

    let b8 = by("scalar_emits_gen_wide_batch8k");
    assert!(
        b8.sql.contains("bench.gen_wide_batch(62500, 1, 8192)"),
        "{}",
        b8.sql
    );
    assert_eq!(b8.expect_count, Some(62_500));
    let b64 = by("scalar_emits_gen_wide_batch64k");
    assert!(
        b64.sql.contains("bench.gen_wide_batch(62500, 1, 65536)"),
        "{}",
        b64.sql
    );
    assert!(
        c.iter()
            .all(|x| x.name != "scalar_emits_gen_wide_batch64k_noemit")
    );
    let no = by("scalar_emits_gen_wide_batch8k_noemit");
    assert!(no.sql.contains("bench.gen_wide_batch(62500, 0, 8192)"));
    assert_eq!(no.expect_count, Some(1));
    assert!(no.wire_bytes_per_row.is_none(), "no MB/s without transfer");
    let row = by("set_gen_wide_row");
    assert!(row.sql.contains("bench.setgen_wide_row(62500, 1)"));
    assert!(
        by("set_gen_wide_batch8k")
            .sql
            .contains("bench.setgen_wide_batch(62500, 1, 8192)")
    );

    // The narrow generators keep their two-parameter call and the full n.
    assert!(
        by("scalar_emits_gen_native_batch")
            .sql
            .contains("gen_native_batch(250000, 1)")
    );
    let ddl = Class::Wide.columns();
    assert!(ddl.starts_with("k DECIMAL(18,0), ") && ddl.contains("s200 VARCHAR(200)"));
    let scripts = scripts("/b/x.so", false);
    let wide_batch = scripts
        .iter()
        .find(|s| s.contains("SCRIPT bench.gen_wide_batch("))
        .unwrap();
    assert!(wide_batch.contains("batch_rows DECIMAL(18,0)) EMITS (k DECIMAL(18,0), "));
}
