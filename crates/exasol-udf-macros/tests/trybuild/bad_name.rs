// Should fail to compile: `name = "BAD NAME"` contains a space.
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

#[exasol_udf(name = "BAD NAME")]
pub fn my_udf(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn main() {}
