// Should compile successfully
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

#[exasol_udf]
pub fn my_udf(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn main() {}
