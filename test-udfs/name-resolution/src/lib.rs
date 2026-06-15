use std::net::ToSocketAddrs;

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// SET UDF that resolves the external hostname `www.exasol.com` via
/// `getaddrinfo`/`ToSocketAddrs` and emits the first returned IP as a string.
///
/// DNS-only: no CONNECTION object is read, no connect-back session is opened.
/// Resolution failure is a hard `UdfError` — it must not silently mask a DNS
/// misconfiguration in the Alpine SLC image.
#[exasol_udf]
pub fn name_resolution(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}
    let resolved_ip = "www.exasol.com:80"
        .to_socket_addrs()
        .map_err(|e| {
            UdfError::User(format!(
                "name_resolution: failed to resolve www.exasol.com: {e}"
            ))
        })?
        .next()
        .map(|sa| sa.ip().to_string())
        .ok_or_else(|| {
            UdfError::User("name_resolution: no addresses returned for www.exasol.com".into())
        })?;
    ctx.emit(&[Value::String(resolved_ip)])
}
