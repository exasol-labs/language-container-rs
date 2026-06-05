// Should fail to compile: `Vec<u8>` has no ExaType mapping.
#![allow(unused_imports)]
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

#[exasol_udf(input(blob: Vec<u8>), emits(result: i64))]
pub fn bad_udf(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn main() {}
