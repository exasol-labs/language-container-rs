//! Fixture for the `IMPORT ... FROM SCRIPT` and `EXPORT ... INTO SCRIPT`
//! spec-generation hooks, exporting all four scripts from one cdylib.
//!
//! `IMPORT_SPEC_GEN` and `EXPORT_SPEC_GEN` parse their `json_spec` through the
//! SDK's `import` and `export` features rather than a hand-rolled parser, and
//! each returns a `SELECT` that names its worker qualified by
//! `ctx.script_schema()` — so the statement only succeeds when the hook ran
//! against a live context.
//!
//! The two workers differ in how their summary reaches the client: `IMPORT`
//! inserts the generated `SELECT`'s rows, so `IMPORT_WORKER` emits the summary,
//! while `EXPORT` discards them, leaving `EXPORT_WORKER` the error channel. The
//! derived table `IMPORT_SPEC_GEN` builds aliases and casts both columns, so
//! `IMPORT_WORKER`'s variadic input has a fixed type shape to report back —
//! the aliases themselves do not survive as its reported column names.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::spec::{ExportSpec, ImportSpec, Parameter};
use exasol_udf_sdk::value::Value;

/// Stands in for an optional the database did not report, so an omitted field
/// and an empty one stay distinguishable in the one-line summary.
const ABSENT: &str = "<none>";

#[exasol_udf(import_spec(import_sql))]
pub fn import_spec_gen(_ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    Ok(None)
}

#[exasol_udf(export_spec(export_sql))]
pub fn export_spec_gen(_ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    Ok(None)
}

/// Emits the spec summary the hook passed plus the input schema this call
/// actually ran with. Registered variadic and carrying no `input(...)`
/// annotation, so the schema can only come from the runtime accessors.
#[exasol_udf]
pub fn import_worker(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let summary = ctx.get_string(0)?.unwrap_or_default().to_string();
    let schema = runtime_schema(ctx)?;
    ctx.emit(vec![Value::String(summary), Value::String(schema)])
}

#[exasol_udf]
pub fn export_worker(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    let summary = ctx.get_string(0)?.unwrap_or_default();
    Err(UdfError::User(format!("EXPORT_SPEC {summary}")))
}

fn import_sql(ctx: &mut dyn UdfContext, json_spec: &str) -> Result<String, UdfError> {
    let spec = ImportSpec::from_json(json_spec)?;
    let columns: Vec<&str> = spec
        .subselect_column_specification
        .iter()
        .map(|column| column.name.as_str())
        .collect();
    let summary = format!(
        "conn={} params=[{}] is_subselect={} cols=[{}]",
        spec.connection_name.as_deref().unwrap_or(ABSENT),
        render(&spec.parameters),
        spec.is_subselect,
        columns.join(","),
    );
    Ok(format!(
        "SELECT {schema}.IMPORT_WORKER(SPEC, PARAM_COUNT) FROM (SELECT \
         CAST('{summary}' AS VARCHAR(2000)) AS SPEC, \
         CAST({count} AS DECIMAL(9,0)) AS PARAM_COUNT)",
        schema = ctx.script_schema(),
        summary = quote(&summary),
        count = spec.parameters.len(),
    ))
}

fn export_sql(ctx: &mut dyn UdfContext, json_spec: &str) -> Result<String, UdfError> {
    let spec = ExportSpec::from_json(json_spec)?;
    let summary = format!(
        "conn={} params=[{}] has_truncate={} has_replace={} cols=[{}]",
        spec.connection_name.as_deref().unwrap_or(ABSENT),
        render(&spec.parameters),
        spec.has_truncate,
        spec.has_replace,
        spec.source_column_names.join(","),
    );
    Ok(format!(
        "SELECT {schema}.EXPORT_WORKER('{summary}')",
        schema = ctx.script_schema(),
        summary = quote(&summary),
    ))
}

fn runtime_schema(ctx: &dyn UdfContext) -> Result<String, UdfError> {
    let count = ctx.input_column_count();
    let mut described = Vec::with_capacity(count);
    for idx in 0..count {
        let column = ctx.input_column(idx)?;
        described.push(format!("{}:{}", column.name, column.type_name));
    }
    Ok(format!("cols={count} [{}]", described.join(",")))
}

fn render(parameters: &[Parameter]) -> String {
    parameters
        .iter()
        .map(|parameter| format!("{}={}", parameter.key, parameter.value))
        .collect::<Vec<_>>()
        .join(",")
}

/// Escape for a SQL single-quoted literal: a `WITH` parameter value is whatever
/// the statement's author typed.
fn quote(text: &str) -> String {
    text.replace('\'', "''")
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
