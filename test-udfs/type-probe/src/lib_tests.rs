use super::*;
use exasol_udf_sdk::test_support::TestContext;
use exasol_udf_sdk::value::{ColumnInfo, Decimal, ExaType};

fn input_col(name: &str, typ: ExaType, type_name: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        typ,
        type_name: type_name.into(),
        size: None,
        precision: None,
        scale: None,
    }
}

fn output_cols(typs: &[(&str, ExaType, &str)]) -> Vec<ColumnInfo> {
    let mut cols: Vec<ColumnInfo> = typs
        .iter()
        .map(|(name, typ, tn)| input_col(name, typ.clone(), tn))
        .collect();
    cols.push(input_col(
        "diag",
        ExaType::String { size: 2000 },
        "VARCHAR(2000)",
    ));
    cols
}

#[test]
fn round_trips_int32() {
    let mut ctx = TestContext::set(vec![vec![Value::Int32(42)]])
        .with_input_columns(vec![input_col("x", ExaType::Int32, "DECIMAL(9,0)")])
        .with_output_columns(output_cols(&[("y", ExaType::Int32, "DECIMAL(9,0)")]));
    type_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::Int32(42));
    let diag = match &rows[0][1] {
        Value::String(s) => s.as_str(),
        _ => panic!("expected diag string"),
    };
    assert!(diag.contains("Int32"));
    assert!(diag.contains("DECIMAL(9,0)"));
}

#[test]
fn round_trips_double() {
    let mut ctx = TestContext::set(vec![vec![Value::Double(42.5)]])
        .with_input_columns(vec![input_col("x", ExaType::Double, "DOUBLE")])
        .with_output_columns(output_cols(&[("y", ExaType::Double, "DOUBLE")]));
    type_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows[0][0], Value::Double(42.5));
    assert!(matches!(&rows[0][1], Value::String(s) if s.contains("Double")));
}

#[test]
fn round_trips_bool() {
    let mut ctx = TestContext::set(vec![vec![Value::Bool(true)]])
        .with_input_columns(vec![input_col("x", ExaType::Boolean, "BOOLEAN")])
        .with_output_columns(output_cols(&[("y", ExaType::Boolean, "BOOLEAN")]));
    type_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows[0][0], Value::Bool(true));
    assert!(matches!(&rows[0][1], Value::String(s) if s.contains("Bool")));
}

#[test]
fn round_trips_numeric() {
    let val = Value::Numeric(Decimal {
        unscaled: 1234,
        scale: 2,
    });
    let mut ctx = TestContext::set(vec![vec![val.clone()]])
        .with_input_columns(vec![input_col(
            "x",
            ExaType::Numeric {
                precision: 18,
                scale: 2,
            },
            "DECIMAL(18,2)",
        )])
        .with_output_columns(output_cols(&[(
            "y",
            ExaType::Numeric {
                precision: 18,
                scale: 2,
            },
            "DECIMAL(18,2)",
        )]));
    type_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows[0][0], val);
    assert!(matches!(&rows[0][1], Value::String(s) if s.contains("Numeric")));
}

#[test]
fn null_emits_null_with_diag() {
    let mut ctx = TestContext::set(vec![vec![Value::Null]])
        .with_input_columns(vec![input_col("x", ExaType::Int64, "DECIMAL(18,0)")])
        .with_output_columns(output_cols(&[("y", ExaType::Int64, "DECIMAL(18,0)")]));
    type_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows[0][0], Value::Null);
    assert!(matches!(&rows[0][1], Value::String(s) if s.contains("Null")));
}

#[test]
fn multiple_columns() {
    let mut ctx = TestContext::set(vec![vec![Value::Int32(1), Value::Double(2.0)]])
        .with_input_columns(vec![
            input_col("a", ExaType::Int32, "DECIMAL(9,0)"),
            input_col("b", ExaType::Double, "DOUBLE"),
        ])
        .with_output_columns(output_cols(&[
            ("a", ExaType::Int32, "DECIMAL(9,0)"),
            ("b", ExaType::Double, "DOUBLE"),
        ]));
    type_probe(&mut ctx).unwrap();
    let rows = ctx.emitted();
    assert_eq!(rows[0][0], Value::Int32(1));
    assert_eq!(rows[0][1], Value::Double(2.0));
    let diag = match &rows[0][2] {
        Value::String(s) => s.as_str(),
        _ => panic!("expected diag string"),
    };
    assert!(diag.contains("a:Int32"));
    assert!(diag.contains("b:Double"));
}
