// Should fail to compile (duplicate __exa_udf_entry_DUP symbol because both
// annotations resolve to the same SQL name "DUP" via `name = "DUP"`).
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

#[exasol_udf(name = "DUP")]
pub fn udf_one(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf(name = "DUP")]
pub fn udf_two(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn main() {}
