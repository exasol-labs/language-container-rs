use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

/// Rendered in place of an identity field the database did not report. The five
/// fields cross the wire as one text column, so an omitted field and an empty
/// one would otherwise look alike; the literal keeps them distinguishable.
const ABSENT_FIELD: &str = "<none>";

/// Scalar UDF returning the session identity metadata as one pipe-delimited
/// string: `current_user|scope_user|current_schema|script_schema|script_name`.
///
/// The database fills the first three from live session state, so only a live
/// round trip can observe them. This fixture is that observation point: it
/// reads all five through the `UdfContext` accessors and hands them back
/// unaltered, rendering an absent optional as [`ABSENT_FIELD`]. Reads metadata
/// only; opens no connect-back session.
#[exasol_udf]
pub fn current_user_meta(ctx: &mut dyn UdfContext) -> Result<Option<String>, UdfError> {
    let summary = format!(
        "{}|{}|{}|{}|{}",
        render(ctx.current_user()),
        render(ctx.scope_user()),
        render(ctx.current_schema()),
        ctx.script_schema(),
        ctx.script_name(),
    );
    Ok(Some(summary))
}

fn render(field: Option<String>) -> String {
    field.unwrap_or_else(|| ABSENT_FIELD.to_string())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
