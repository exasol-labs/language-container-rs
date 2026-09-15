use super::*;
use exasol_udf_sdk::test_support::TestContext;
use exasol_udf_sdk::value::ColumnInfo;

fn output_columns(precision: u32) -> Vec<ColumnInfo> {
    vec![
        ColumnInfo {
            name: "ts".into(),
            typ: ExaType::Timestamp { precision },
            type_name: format!("TIMESTAMP({precision})"),
            size: None,
            precision: Some(precision),
            scale: None,
        },
        ColumnInfo {
            name: "prec".into(),
            typ: ExaType::Int32,
            type_name: "INTEGER".into(),
            size: None,
            precision: None,
            scale: None,
        },
    ]
}

#[test]
fn reports_precision_3() {
    let mut ctx = TestContext::set(vec![vec![Value::Null]]).with_output_columns(output_columns(3));
    ts_precision_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], Value::Int32(3));
}

#[test]
fn reports_precision_6() {
    let mut ctx = TestContext::set(vec![vec![Value::Null]]).with_output_columns(output_columns(6));
    ts_precision_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows[0][1], Value::Int32(6));
}

#[test]
fn reports_precision_9() {
    let mut ctx = TestContext::set(vec![vec![Value::Null]]).with_output_columns(output_columns(9));
    ts_precision_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows[0][1], Value::Int32(9));
    let ts = match &rows[0][0] {
        Value::Timestamp(ts) => ts,
        _ => panic!("expected timestamp"),
    };
    assert_eq!(ts.and_utc().timestamp_subsec_nanos(), 123_456_789);
}
