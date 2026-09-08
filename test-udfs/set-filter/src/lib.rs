use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn set_filter(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {
        match ctx.get(0)? {
            Value::Int64(n) if *n > 0 => ctx.emit(&[Value::Int64(*n)])?,
            // Exasol sends BIGINT as PB_NUMERIC (typed Decimal with scale=0).
            Value::Numeric(d) if d.scale == 0 => {
                let n = i64::try_from(d.unscaled)
                    .map_err(|_| UdfError::Type(format!("cannot convert {} to i64", d)))?;
                if n > 0 {
                    ctx.emit(&[Value::Numeric(Decimal {
                        unscaled: n as i128,
                        scale: 0,
                    })])?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exasol_udf_sdk::test_support::TestContext;

    #[test]
    fn emits_only_positive_rows() {
        let mut ctx = TestContext::set(vec![
            vec![Value::Int64(-1)],
            vec![Value::Int64(0)],
            vec![Value::Int64(3)],
            vec![Value::Int64(7)],
        ]);
        set_filter(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted(),
            vec![vec![Value::Int64(3)], vec![Value::Int64(7)]]
        );
    }

    #[test]
    fn emits_nothing_for_all_non_positive() {
        let mut ctx = TestContext::set(vec![vec![Value::Int64(-5)], vec![Value::Int64(0)]]);
        set_filter(&mut ctx).unwrap();
        assert!(ctx.emitted().is_empty());
    }

    #[test]
    fn handles_empty_input() {
        let mut ctx = TestContext::set(vec![]);
        set_filter(&mut ctx).unwrap();
        assert!(ctx.emitted().is_empty());
    }

    #[test]
    fn skips_null_rows() {
        let mut ctx = TestContext::set(vec![vec![Value::Null], vec![Value::Int64(5)]]);
        set_filter(&mut ctx).unwrap();
        assert_eq!(ctx.emitted(), vec![vec![Value::Int64(5)]]);
    }
}
