use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::net::ToSocketAddrs;

#[exasol_udf]
pub fn resolv_udf(ctx: &mut dyn UdfContext) -> Result<Option<String>, UdfError> {
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
    Ok(Some(addr.ip().to_string()))
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
    fn resolves_localhost_to_ip() {
        let mut ctx = returns_ctx(vec![Value::String("localhost".into())]);
        let result = resolv_udf(&mut ctx).unwrap();
        let ip = result.expect("resolv_udf should return Some(ip)");
        ip.parse::<std::net::IpAddr>()
            .expect("returned value should be a valid IP address");
    }

    #[test]
    fn errors_on_unresolvable_host() {
        let mut ctx = returns_ctx(vec![Value::String(
            "this-host-definitely-does-not-exist.invalid".into(),
        )]);
        let err = resolv_udf(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::User(_)));
    }

    #[test]
    fn errors_on_non_string_input() {
        let mut ctx = returns_ctx(vec![Value::Int64(42)]);
        let err = resolv_udf(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::Type(_)));
    }
}
