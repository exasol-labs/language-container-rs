//! Fixture proving `ctx.rows_in_group()` is available before the group's
//! first `next()` call, and reports the same count throughout the group.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// Reads `ctx.rows_in_group()` before iterating, then iterates the whole
/// group with `next()`, emitting one row that carries the group key, the
/// reported count, and the number of rows actually iterated.
#[exasol_udf]
pub fn rows_in_group(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let reported = ctx.rows_in_group() as i64;

    let mut group_key: i64 = 0;
    let mut iterated: i64 = 0;
    while ctx.next()? {
        if iterated == 0 {
            group_key = ctx.get_i64(0)?.unwrap_or_default();
        }
        iterated += 1;
    }

    ctx.emit(vec![
        Value::Int64(group_key),
        Value::Int64(reported),
        Value::Int64(iterated),
    ])
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
