use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use serde_json::Value as JsonValue;

#[exasol_udf]
pub fn json_parse(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let json_str = match ctx.get(0)? {
        Value::String(s) => s.clone(),
        Value::Null => return ctx.emit(&[Value::Null]),
        _ => return Err(UdfError::Type("expected String".into())),
    };
    let parsed: JsonValue = serde_json::from_str(&json_str)
        .map_err(|e| UdfError::User(format!("JSON parse error: {}", e)))?;
    let name = parsed["name"].as_str().unwrap_or("").to_string();
    ctx.emit(&[Value::String(name)])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        input: Vec<Value>,
        emitted: Vec<Vec<Value>>,
    }

    impl TestCtx {
        fn new(row: Vec<Value>) -> Self {
            Self {
                input: row,
                emitted: Vec::new(),
            }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize {
            self.input.len()
        }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.input
                .get(col)
                .ok_or_else(|| UdfError::User(format!("col {} out of range", col)))
        }

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
    }

    #[test]
    fn extracts_name_field() {
        let mut ctx = TestCtx::new(vec![Value::String(r#"{"name":"exa"}"#.into())]);
        json_parse(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::String("exa".into())]]);
    }

    #[test]
    fn returns_empty_string_when_name_absent() {
        let mut ctx = TestCtx::new(vec![Value::String(r#"{"other":"val"}"#.into())]);
        json_parse(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::String("".into())]]);
    }

    #[test]
    fn passes_null_through() {
        let mut ctx = TestCtx::new(vec![Value::Null]);
        json_parse(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Null]]);
    }

    #[test]
    fn errors_on_invalid_json() {
        let mut ctx = TestCtx::new(vec![Value::String("not json".into())]);
        let err = json_parse(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::User(_)));
    }

    #[test]
    fn rejects_wrong_type() {
        let mut ctx = TestCtx::new(vec![Value::Int64(42)]);
        let err = json_parse(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::Type(_)));
    }
}
