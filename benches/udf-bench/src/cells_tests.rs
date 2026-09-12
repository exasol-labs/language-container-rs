use std::collections::HashSet;

use super::*;

fn by<'a>(c: &'a [CellSpec], name: &str) -> &'a CellSpec {
    c.iter().find(|x| x.name == name).unwrap()
}

#[test]
fn matrix_is_44_unique_cells_with_resolving_controls_and_passthrough_last() {
    let c = cells(250_000);
    assert_eq!(c.len(), 44);
    let names: HashSet<&str> = c.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names.len(), 44);
    assert_eq!(c.iter().filter(|x| x.shape == "control").count(), 3);
    for x in c.iter().filter(|x| x.control.is_some()) {
        assert!(names.contains(x.control.as_deref().unwrap()), "{}", x.name);
        assert_ne!(x.shape, "control");
    }
    let pt = c.last().unwrap();
    assert!(pt.passthrough);
    assert_eq!(pt.rows, 250_000);
    assert!(pt.sql.contains("pt_native(k, 250)"));
    assert_eq!(pt.expect_count, Some(250_000));
}

#[test]
fn every_script_is_used_by_a_cell() {
    let list = scripts("/b/x.so", false);
    assert_eq!(list.len(), 3 + 3 * 2 * 3 + 4 + 1 + 2);
    let all_sql: String = cells(10).iter().map(|c| c.sql.clone()).collect();
    for s in &list {
        assert!(s.ends_with("%udf_object /b/x.so;\n/"), "{s}");
        let name = s
            .split("SCRIPT bench.")
            .nth(1)
            .unwrap()
            .split('(')
            .next()
            .unwrap();
        if !name.starts_with("set_emit_varchar") {
            assert!(
                all_sql.contains(&format!("bench.{name}(")),
                "unused script {name}"
            );
        }
    }
    let wide_batch = list
        .iter()
        .find(|s| s.contains("bench.gen_wide_batch("))
        .unwrap();
    assert!(wide_batch.contains("batch_rows DECIMAL(18,0)) EMITS (k DECIMAL(18,0), "));
    assert!(
        scripts("/b/x.so", true)
            .iter()
            .all(|s| s.ends_with("%udf_object /b/x.so;\n%udf_debug_level debug;\n/"))
    );
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
fn strblock_input_cells_read_a_smaller_table() {
    assert_eq!(Class::Strblock.input_rows(250_000), 2_500);
    assert_eq!(Class::Strblock.input_rows(1_000_000), 10_000);
    assert_eq!(Class::Strblock.input_rows(10_000), 1_000);
    assert_eq!(Class::Strblock.input_rows(500), 500);
    assert_eq!(Class::Native.input_rows(250_000), 250_000);

    let c = cells(250_000);
    for name in [
        "control_strblock",
        "scalar_returns_strblock",
        "set_returns_strblock_g1",
        "set_emits_strblock_batch_g1000",
    ] {
        assert_eq!(by(&c, name).rows, 2_500, "{name}");
    }
    assert_eq!(by(&c, "scalar_returns_strblock").expect_count, Some(2_500));
    assert_eq!(
        by(&c, "set_returns_strblock_g1000").expect_count,
        Some(1_000)
    );
    assert_eq!(
        by(&c, "set_emits_strblock_row_g1").expect_count,
        Some(2_500)
    );
    assert_eq!(by(&c, "scalar_emits_gen_strblock_row").rows, 250_000);
    assert_eq!(by(&c, "set_gen_strblock_batch").expect_count, Some(250_000));
    assert_eq!(by(&c, "scalar_returns_native").rows, 250_000);
}

#[test]
fn generator_cells_noemit_twins_and_wide_batch_rows() {
    let c = cells(250_000);
    let no = by(&c, "scalar_emits_gen_native_batch_noemit");
    assert!(no.sql.contains("bench.gen_native_batch(250000, 0)"));
    assert_eq!(no.expect_count, Some(1));
    assert!(no.wire_bytes_per_row.is_none());
    let yes = by(&c, "scalar_emits_gen_native_batch");
    assert!(yes.sql.contains("bench.gen_native_batch(250000, 1)"));
    assert_eq!(yes.expect_count, Some(250_000));
    assert_eq!(yes.wire_bytes_per_row, Some(12.9));

    let wide: Vec<&CellSpec> = c.iter().filter(|x| x.class == Some("wide")).collect();
    assert_eq!(wide.len(), 7);
    assert!(wide.iter().all(|x| x.rows == 250_000 / WIDE_GEN_DIVISOR));
    assert!(
        wide.iter()
            .all(|x| x.shape == "scalar_emits_gen" || x.shape == "set_gen")
    );
    assert!(
        by(&c, "scalar_emits_gen_wide_batch8k")
            .sql
            .contains("bench.gen_wide_batch(62500, 1, 8192)")
    );
    assert!(
        by(&c, "scalar_emits_gen_wide_batch64k")
            .sql
            .contains("bench.gen_wide_batch(62500, 1, 65536)")
    );
    assert!(
        by(&c, "scalar_emits_gen_wide_batch8k_noemit")
            .sql
            .contains("bench.gen_wide_batch(62500, 0, 8192)")
    );
    assert!(
        c.iter()
            .all(|x| x.name != "scalar_emits_gen_wide_batch64k_noemit")
    );
    assert!(
        by(&c, "set_gen_wide_row")
            .sql
            .contains("bench.setgen_wide_row(62500, 1)")
    );
    assert!(
        by(&c, "set_gen_wide_batch8k")
            .sql
            .contains("bench.setgen_wide_batch(62500, 1, 8192)")
    );
    let ddl = Class::Wide.columns();
    assert!(ddl.starts_with("k DECIMAL(18,0), ") && ddl.contains("s200 VARCHAR(200)"));
}
