use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn set_filter(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {
        match ctx.get(0)? {
            Value::Int64(n) if *n > 0 => ctx.emit(&[Value::Int64(*n)])?,
            // Exasol sends BIGINT as PB_NUMERIC (decimal string).
            Value::Numeric(s) => {
                let n: i64 = s
                    .parse()
                    .map_err(|e| UdfError::Type(format!("cannot parse '{}' as i64: {}", s, e)))?;
                if n > 0 {
                    ctx.emit(&[Value::Numeric(n.to_string())])?;
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

    struct TestCtx {
        rows: Vec<Vec<Value>>,
        cursor: usize,
        emitted: Vec<Vec<Value>>,
    }

    impl TestCtx {
        fn new(rows: Vec<Vec<Value>>) -> Self {
            Self {
                rows,
                cursor: 0,
                emitted: Vec::new(),
            }
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

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
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
    fn emits_only_positive_rows() {
        let mut ctx = TestCtx::new(vec![
            vec![Value::Int64(-1)],
            vec![Value::Int64(0)],
            vec![Value::Int64(3)],
            vec![Value::Int64(7)],
        ]);
        set_filter(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted,
            vec![vec![Value::Int64(3)], vec![Value::Int64(7)]]
        );
    }

    #[test]
    fn emits_nothing_for_all_non_positive() {
        let mut ctx = TestCtx::new(vec![vec![Value::Int64(-5)], vec![Value::Int64(0)]]);
        set_filter(&mut ctx).unwrap();
        assert!(ctx.emitted.is_empty());
    }

    #[test]
    fn handles_empty_input() {
        let mut ctx = TestCtx::new(vec![]);
        set_filter(&mut ctx).unwrap();
        assert!(ctx.emitted.is_empty());
    }

    #[test]
    fn skips_null_rows() {
        let mut ctx = TestCtx::new(vec![vec![Value::Null], vec![Value::Int64(5)]]);
        set_filter(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Int64(5)]]);
    }
}
