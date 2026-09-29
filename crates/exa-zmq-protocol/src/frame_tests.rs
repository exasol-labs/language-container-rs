use super::*;
use exa_proto::ExascriptEmitDataReq;

fn prost_frame(connection_id: u64, table: &EmitTable) -> Vec<u8> {
    ExascriptRequest {
        r#type: MessageType::MtEmit as i32,
        connection_id,
        emit: Some(ExascriptEmitDataReq {
            table: table.to_proto(),
        }),
        ..Default::default()
    }
    .encode_to_vec()
}

fn assert_byte_identical(connection_id: u64, table: &EmitTable) {
    let frame = EmitRequest {
        connection_id,
        table,
    }
    .encode_frame();
    assert_eq!(frame, prost_frame(connection_id, table));
    let decoded = ExascriptRequest::decode(frame.as_slice()).unwrap();
    assert_eq!(decoded.emit.unwrap().table, table.to_proto());
}

fn push_string(t: &mut EmitTable, s: &[u8]) {
    if s.len().is_multiple_of(2) {
        t.push_string(s);
    } else {
        t.push_string_with(|out| out.extend_from_slice(s));
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

#[test]
fn empty_table_matches_prost() {
    assert_byte_identical(0, &EmitTable::default());
}

#[test]
fn empty_strings_and_nulls_match_prost() {
    let mut t = EmitTable {
        rows: 2,
        nulls: vec![false, true, false, false],
        int64: vec![-1],
        row_numbers: vec![7, u64::MAX],
        ..Default::default()
    };
    push_string(&mut t, b"");
    push_string(&mut t, "héllo".as_bytes());
    assert_byte_identical(42, &t);
}

#[test]
fn single_cell_over_the_flush_target_matches_prost() {
    let mut t = EmitTable {
        rows: 1,
        nulls: vec![false, false],
        row_numbers: vec![0],
        ..Default::default()
    };
    t.push_string_with(|out| out.resize(out.len() + 4_000_001, b'x'));
    t.push_string(&vec![b'y'; 4_000_001]);
    assert_byte_identical(1, &t);
}

#[test]
fn random_tables_match_prost() {
    let mut rng = Lcg(1);
    for _ in 0..500 {
        let mut t = EmitTable {
            rows: rng.next(1 << 40),
            ..Default::default()
        };
        for _ in 0..rng.next(20) {
            let bound = if rng.next(10) == 0 { 20_000 } else { 300 };
            let len = rng.next(bound) as usize;
            push_string(&mut t, &vec![b'a' + (len % 26) as u8; len]);
        }
        t.nulls = (0..rng.next(40)).map(|_| rng.next(2) == 0).collect();
        t.bools = (0..rng.next(10)).map(|_| rng.next(2) == 0).collect();
        t.int32 = (0..rng.next(10))
            .map(|_| rng.next(u64::MAX) as i32)
            .collect();
        t.int64 = (0..rng.next(10))
            .map(|_| rng.next(u64::MAX) as i64 - (1 << 40))
            .collect();
        t.doubles = (0..rng.next(10))
            .map(|_| rng.next(1 << 50) as f64 / 3.0)
            .collect();
        t.row_numbers = (0..rng.next(20)).map(|_| rng.next(1 << 50)).collect();
        assert_byte_identical(rng.next(u64::MAX), &t);
    }
}
