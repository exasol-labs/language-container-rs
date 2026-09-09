//! Doubles a BIGINT input.
//!
//! `i64::MAX` is a test-only sentinel that panics, covering the runtime's
//! rc!=0-without-out-pointer path. It cannot fire against a live DB, which
//! delivers BIGINT as PB_NUMERIC rather than Int64.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    let doubled = match ctx.get(0)? {
        Value::Int64(i64::MAX) => {
            panic!("scalar-double fixture: deliberate panic for the no-out-pointer dispatch path")
        }
        Value::Int64(n) => Value::Int64(n * 2),
        // Exasol sends BIGINT as PB_NUMERIC (typed Decimal with scale=0).
        Value::Numeric(d) if d.scale == 0 => {
            let n = i64::try_from(d.unscaled)
                .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))?;
            Value::Numeric(Decimal {
                unscaled: (n * 2) as i128,
                scale: 0,
            })
        }
        Value::Null => return Ok(None),
        _ => return Err(UdfError::Type("expected Int64 or Numeric".into())),
    };
    Ok(Some(doubled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exasol_udf_sdk::test_support::{EmitPolicy, TestContext};

    fn returns_ctx(row: Vec<Value>) -> TestContext {
        TestContext::scalar(row).with_emit_policy(EmitPolicy::Reject(UdfError::Unimplemented(
            "emit is banned in RETURNS output".into(),
        )))
    }

    #[test]
    fn doubles_positive_int64() {
        let mut ctx = returns_ctx(vec![Value::Int64(21)]);
        let result = scalar_double(&mut ctx).unwrap();
        assert_eq!(result, Some(Value::Int64(42)));
    }

    #[test]
    fn doubles_negative_int64() {
        let mut ctx = returns_ctx(vec![Value::Int64(-5)]);
        let result = scalar_double(&mut ctx).unwrap();
        assert_eq!(result, Some(Value::Int64(-10)));
    }

    #[test]
    fn passes_null_through() {
        let mut ctx = returns_ctx(vec![Value::Null]);
        let result = scalar_double(&mut ctx).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn rejects_wrong_type() {
        let mut ctx = returns_ctx(vec![Value::String("x".into())]);
        let err = scalar_double(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::Type(_)));
    }
}
