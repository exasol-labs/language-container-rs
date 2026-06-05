//! Test fixture with a macro-emitted annotated schema.
//!
//! The schema (`input(x: i64)`, `emits(y: i64)`) is baked into the vtable by the
//! `#[exasol_udf]` macro so the runtime's load-time schema validation can be
//! tested against a real `.so`.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf(input(x: i64), emits(y: i64))]
pub fn annotated(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let v = match ctx.get(0)? {
        Value::Int64(n) => Value::Int64(*n),
        other => other.clone(),
    };
    ctx.emit(&[v])
}
