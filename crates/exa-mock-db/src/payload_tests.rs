use super::*;

fn decode_frame(frame: &[u8]) -> ExascriptTableData {
    ExascriptResponse::decode(frame)
        .unwrap()
        .next
        .unwrap()
        .table
}

fn table_cost(t: &ExascriptTableData) -> usize {
    (t.data_int64.len() + t.data_double.len()) * FIXED_CELL_BYTES
        + t.data_string.iter().map(String::len).sum::<usize>()
}

fn frames(enc: &EncodedInput) -> usize {
    enc.cycles.iter().map(Vec::len).sum()
}

#[test]
fn classes_declare_their_wire_types() {
    let native = ColumnClass::Native.columns();
    assert_eq!(native[0].r#type, Some(ColumnType::PbInt64 as i32));
    assert_eq!(native[1].r#type, Some(ColumnType::PbDouble as i32));
    let sb = ColumnClass::Strblock.columns();
    assert_eq!(sb.len(), 4);
    assert_eq!(sb[1].r#type, Some(ColumnType::PbNumeric as i32));
    assert_eq!(sb[1].scale, Some(2));
    assert_eq!(sb[2].r#type, Some(ColumnType::PbDate as i32));
    assert_eq!(sb[3].r#type, Some(ColumnType::PbTimestamp as i32));
    let vc = ColumnClass::Varchar.columns();
    assert_eq!(vc[1].r#type, Some(ColumnType::PbString as i32));
    assert_eq!(vc[1].size, Some(100));
    let wide = ColumnClass::Wide.columns();
    assert_eq!(wide.len(), 24);
    assert_eq!(wide[6].type_name, "BOOLEAN");
    assert_eq!((wide[10].precision, wide[10].scale), (Some(36), Some(10)));
    assert_eq!(wide[23].size, Some(200));
}

#[test]
fn strblock_row_lands_in_the_right_blocks_with_database_rendering() {
    let mut t = ExascriptTableData::default();
    let cost = ColumnClass::Strblock.write_row(1, &mut t);
    assert_eq!(t.data_int64, vec![1]);
    assert_eq!(
        t.data_string,
        vec!["43.37", "2020-01-02", "2020-01-01 00:00:01.000137"]
    );
    assert_eq!(t.data_nulls, vec![false; 4]);
    assert_eq!(cost, 8 + 5 + 10 + 26);
    assert_eq!(amount_text(0), "42.00");
    assert_eq!(date_text(3650), "2020-01-01");
}

#[test]
fn every_batch_is_at_most_the_limit_plus_one_row() {
    let rows = 100_000u64;
    let enc = encode(&ColumnClass::Varchar, rows, 1);
    let frames = &enc.cycles[0];
    assert!(frames.len() >= 2);
    let mut total_rows = 0;
    for f in frames {
        let t = decode_frame(f);
        assert!(table_cost(&t) <= EMIT_LIMIT_BYTES + 8 + 50);
        total_rows += t.rows;
    }
    assert_eq!(total_rows, rows);
    for f in &frames[..frames.len() - 1] {
        assert!(table_cost(&decode_frame(f)) > EMIT_LIMIT_BYTES);
    }
}

#[test]
fn row_numbers_are_monotonic_across_cycles_and_totals_exact() {
    let rows = 1_000u64;
    let enc = encode(&ColumnClass::Native, rows, 7);
    assert_eq!(enc.cycles.len(), 7);
    let mut expected = 0u64;
    let mut ks = Vec::new();
    for t in enc.cycles.iter().flatten().map(|f| decode_frame(f)) {
        assert_eq!(t.row_number.len() as u64, t.rows);
        for rn in &t.row_number {
            assert_eq!(*rn, expected);
            expected += 1;
        }
        ks.extend_from_slice(&t.data_int64);
    }
    assert_eq!(expected, rows);
    assert_eq!(ks, (0..rows as i64).collect::<Vec<_>>());
    let sparse = encode(&ColumnClass::Native, 2, 4);
    assert_eq!(sparse.cycles.len(), 4);
    assert_eq!(frames(&sparse), 2);
}

#[test]
fn int64_columns_fill_per_row() {
    let src = Int64Columns::new(&["n", "do_emit"], |_| vec![250_000, 1]);
    let t = decode_frame(&encode(&src, 1, 1).cycles[0][0]);
    assert_eq!(t.rows, 1);
    assert_eq!(t.data_int64, vec![250_000, 1]);
    assert_eq!(src.columns()[1].name, "do_emit");
}

#[test]
fn frame_cursor_yields_frames_then_none() {
    let frames = vec![vec![1u8], vec![2u8]];
    let mut c = FrameCursor::new(&frames);
    assert_eq!(c.next_frame(), Some(&[1u8][..]));
    assert_eq!(c.next_frame(), Some(&[2u8][..]));
    assert_eq!(c.next_frame(), None);
}
