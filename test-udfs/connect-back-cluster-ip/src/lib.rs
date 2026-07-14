use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

/// Scalar UDF that returns the raw node IP of the cluster node that started
/// the language container. Does not open a connect-back session.
#[exasol_udf]
pub fn connect_back_cluster_ip(ctx: &mut dyn UdfContext) -> Result<Option<String>, UdfError> {
    let ip = ctx.cluster_ip()?;
    Ok(Some(ip))
}
