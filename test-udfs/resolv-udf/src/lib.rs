use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::net::ToSocketAddrs;

#[exasol_udf]
pub fn resolv_udf(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let host = match ctx.get(0)? {
        Value::String(s) => s.clone(),
        Value::Null => return Err(UdfError::Type("host must not be NULL".into())),
        _ => return Err(UdfError::Type("expected VARCHAR host".into())),
    };
    let addr = format!("{host}:0")
        .to_socket_addrs()
        .map_err(|e| UdfError::User(format!("DNS resolution failed for {host:?}: {e}")))?
        .next()
        .ok_or_else(|| UdfError::User(format!("no addresses returned for {host:?}")))?;
    ctx.emit(&[Value::String(addr.ip().to_string())])
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
                .ok_or_else(|| UdfError::User(format!("col {col} out of range")))
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
    fn resolves_localhost_to_ip() {
        let mut ctx = TestCtx::new(vec![Value::String("localhost".into())]);
        resolv_udf(&mut ctx).unwrap();
        assert_eq!(ctx.emitted.len(), 1);
        let ip = match &ctx.emitted[0][0] {
            Value::String(s) => s.clone(),
            other => panic!("expected String, got {other:?}"),
        };
        ip.parse::<std::net::IpAddr>()
            .expect("emitted value should be a valid IP address");
    }

    #[test]
    fn errors_on_unresolvable_host() {
        let mut ctx = TestCtx::new(vec![Value::String(
            "this-host-definitely-does-not-exist.invalid".into(),
        )]);
        let err = resolv_udf(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::User(_)));
    }

    #[test]
    fn errors_on_non_string_input() {
        let mut ctx = TestCtx::new(vec![Value::Int64(42)]);
        let err = resolv_udf(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::Type(_)));
    }
}
