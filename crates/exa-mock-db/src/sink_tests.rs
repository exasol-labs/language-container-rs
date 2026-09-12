use super::*;

fn table(rows: u64, with_row_number: bool) -> ExascriptTableData {
    ExascriptTableData {
        rows,
        row_number: if with_row_number {
            (0..rows).collect()
        } else {
            vec![]
        },
        ..Default::default()
    }
}

#[test]
fn counters_track_count_rows_bytes_max_row_number_and_limit() {
    let mut c = EmitCounters::default();
    c.observe(100, &table(3, false));
    c.observe(300, &table(5, true));
    assert_eq!(c.messages, 2);
    assert_eq!(c.rows, 8);
    assert_eq!(c.bytes, 400);
    assert_eq!(c.max_bytes, 300);
    assert_eq!(c.mean_bytes(), 200.0);
    assert_eq!(c.with_row_number, 1);
    assert_eq!(c.over_limit, 0);
    c.observe(EMIT_LIMIT_BYTES, &table(1, false));
    c.observe(EMIT_LIMIT_BYTES + 1, &table(1, false));
    assert_eq!(c.over_limit, 1);
    assert_eq!(EmitCounters::default().mean_bytes(), 0.0);
}

#[test]
fn collector_keeps_tables_in_order() {
    let mut c = EmitCollector::default();
    let mut t = table(2, false);
    t.data_int64 = vec![7, 8];
    c.observe(1, &t);
    let mut t = table(1, false);
    t.data_int64 = vec![9];
    c.observe(1, &t);
    assert_eq!(c.rows(), 3);
    assert_eq!(c.int64_cells(), vec![7, 8, 9]);
}
