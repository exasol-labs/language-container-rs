// Should fail to compile: `spec_import` is not an annotation section.
#![allow(unused_imports)]
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

#[exasol_udf(spec_import(some_fn))]
pub fn bad_udf(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn main() {}
