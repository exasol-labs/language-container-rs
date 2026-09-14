//! Test fixture for the column-metadata accessors and emit validation.
//!
//! `describe_output` builds its row from the call-site `EMITS` list alone, so
//! registering it against two different lists proves the UDF sees the real
//! output schema. `emit_type_mismatch` emits a value no declared column can
//! carry, so the rejection has to reach the SQL user as an error.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{ExaType, Value};

#[exasol_udf]
pub fn describe_output(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut row = Vec::with_capacity(ctx.output_column_count());
    for idx in 0..ctx.output_column_count() {
        let column = ctx.output_column(idx)?;
        row.push(match &column.typ {
            ExaType::String { .. } | ExaType::Char { .. } => {
                Value::String(format!("{}|{}", column.name, column.type_name))
            }
            ExaType::Double => Value::Double(idx as f64),
            ExaType::Int32 | ExaType::Int64 | ExaType::Numeric { .. } => Value::Int64(idx as i64),
            ExaType::Boolean => Value::Bool(true),
            _ => Value::Null,
        });
    }
    ctx.emit(row)
}

#[exasol_udf]
pub fn input_column_name(ctx: &mut dyn UdfContext) -> Result<Option<String>, UdfError> {
    let column = ctx.input_column(0)?;
    Ok(Some(format!("{}|{}", column.name, column.type_name)))
}

#[exasol_udf]
pub fn emit_type_mismatch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    ctx.emit(vec![Value::String("not a number".into())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use exasol_udf_sdk::test_support::TestContext;
    use exasol_udf_sdk::value::ColumnInfo;

    fn column(name: &str, typ: ExaType, type_name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            typ,
            type_name: type_name.into(),
            size: None,
            precision: None,
            scale: None,
        }
    }

    #[test]
    fn describe_output_follows_the_declared_output_shape() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(1)]).with_output_columns(vec![
            column("a", ExaType::String { size: Some(10) }, "VARCHAR(10) UTF8"),
            column("b", ExaType::Int64, "BIGINT"),
        ]);

        describe_output(&mut ctx).unwrap();

        assert_eq!(
            ctx.emitted(),
            &[vec![
                Value::String("a|VARCHAR(10) UTF8".into()),
                Value::Int64(1)
            ]]
        );
    }

    #[test]
    fn input_column_name_reports_the_declared_input_column() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(1)]).with_input_columns(vec![column(
            "x",
            ExaType::Int64,
            "BIGINT",
        )]);

        assert_eq!(
            input_column_name(&mut ctx).unwrap(),
            Some("x|BIGINT".to_string())
        );
    }
}
