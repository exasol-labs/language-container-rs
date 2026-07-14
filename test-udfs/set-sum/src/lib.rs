use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
#[cfg(test)]
use exasol_udf_sdk::value::Value;

/// SET RETURNS fixture: sums the first `i64` (or scale-0 `Numeric`) column over
/// the whole input group via `ctx.next()`, returning the aggregate as a single
/// RETURNS value. Exercises Bug 2 (group-spanning `MT_NEXT` batches) at IT time.
#[exasol_udf]
pub fn set_sum(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    let mut sum: i64 = 0;
    while ctx.next()? {
        if let Some(n) = ctx.get_i64(0)? {
            sum += n;
        }
    }
    Ok(Some(sum))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        rows: Vec<Vec<Value>>,
        cursor: usize,
    }

    impl TestCtx {
        fn new(rows: Vec<Vec<Value>>) -> Self {
            Self { rows, cursor: 0 }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize {
            self.rows.first().map_or(0, |r| r.len())
        }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.rows[self.cursor - 1]
                .get(col)
                .ok_or_else(|| UdfError::User(format!("col {} out of range", col)))
        }

        fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
            Err(UdfError::User("set-sum must not emit".into()))
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            if self.cursor < self.rows.len() {
                self.cursor += 1;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    #[test]
    fn sums_group_of_int64() {
        let mut ctx = TestCtx::new(vec![
            vec![Value::Int64(1)],
            vec![Value::Int64(2)],
            vec![Value::Int64(3)],
        ]);
        assert_eq!(set_sum(&mut ctx).unwrap(), Some(6));
    }

    #[test]
    fn sums_empty_group_to_zero() {
        let mut ctx = TestCtx::new(vec![]);
        assert_eq!(set_sum(&mut ctx).unwrap(), Some(0));
    }

    #[test]
    fn skips_null_rows() {
        let mut ctx = TestCtx::new(vec![vec![Value::Null], vec![Value::Int64(5)]]);
        assert_eq!(set_sum(&mut ctx).unwrap(), Some(5));
    }
}
