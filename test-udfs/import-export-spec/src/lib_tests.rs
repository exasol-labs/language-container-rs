use super::*;
use exasol_udf_sdk::test_support::TestContext;
use exasol_udf_sdk::value::{ColumnInfo, ExaType};

/// The JSON shape `exa-udf-runtime`'s `spec_json` pins, with a duplicate `WITH`
/// key and a single-quoted parameter value the generated SQL has to survive.
const IMPORT_JSON: &str = r#"{
  "is_subselect": true,
  "connection_information": null,
  "connection_name": "IT_IMPORT_CONN",
  "subselect_column_specification": [
    { "name": "SPEC", "type": "PB_STRING", "type_name": "VARCHAR(2000) UTF8",
      "size": 2000, "precision": null, "scale": null },
    { "name": "SCHEMA_INFO", "type": "PB_STRING", "type_name": "VARCHAR(2000) UTF8",
      "size": 2000, "precision": null, "scale": null }
  ],
  "parameters": [
    { "key": "PARAM_A", "value": "it's alpha" },
    { "key": "PARAM_A", "value": "beta" }
  ]
}"#;

const EXPORT_JSON: &str = r#"{
  "has_truncate": true,
  "has_replace": false,
  "created_by": null,
  "source_column_names": ["\"T\".\"K\"", "\"T\".\"LABEL\""],
  "connection_information": null,
  "connection_name": "IT_EXPORT_CONN",
  "parameters": [{ "key": "SHARDS", "value": "4" }]
}"#;

fn column(name: &str) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        typ: ExaType::String { size: 2000 },
        type_name: "VARCHAR(2000) UTF8".into(),
        size: Some(2000),
        precision: None,
        scale: None,
    }
}

#[test]
fn spec_hooks_build_worker_sql_from_json_spec() {
    let mut ctx = TestContext::scalar(Vec::new()).with_script_schema("IT_RUST");

    let sql = import_sql(&mut ctx, IMPORT_JSON).unwrap();
    assert!(
        sql.contains("IT_RUST.IMPORT_WORKER"),
        "the worker must be qualified by the live script schema: {sql}"
    );
    assert!(
        sql.contains("conn=IT_IMPORT_CONN"),
        "the summary must carry the connection name: {sql}"
    );
    assert!(
        sql.contains("params=[PARAM_A=it''s alpha,PARAM_A=beta]"),
        "every WITH parameter must survive in order, with quotes escaped: {sql}"
    );
    assert!(
        sql.contains("is_subselect=true cols=[SPEC,SCHEMA_INFO]"),
        "the summary must carry the subselect flag and declared columns: {sql}"
    );

    let sql = export_sql(&mut ctx, EXPORT_JSON).unwrap();
    assert!(
        sql.contains("IT_RUST.EXPORT_WORKER"),
        "the worker must be qualified by the live script schema: {sql}"
    );
    assert!(
        sql.contains(r#"cols=["T"."K","T"."LABEL"]"#),
        "the summary must carry the source column names verbatim: {sql}"
    );
    assert!(
        sql.contains("has_truncate=true has_replace=false"),
        "the summary must carry both statement flags: {sql}"
    );
}

#[test]
fn spec_hooks_report_an_unreadable_payload() {
    let mut ctx = TestContext::scalar(Vec::new());
    assert!(matches!(
        import_sql(&mut ctx, "{").unwrap_err(),
        UdfError::Type(_)
    ));
    assert!(matches!(
        export_sql(&mut ctx, "{").unwrap_err(),
        UdfError::Type(_)
    ));
}

#[test]
fn worker_builds_row_from_the_runtime_schema() {
    let mut ctx = TestContext::scalar(vec![Value::String("SUMMARY_TEXT".into()), Value::Int64(2)])
        .with_input_columns(vec![column("SPEC"), column("PARAM_COUNT")]);

    import_worker(&mut ctx).unwrap();

    assert_eq!(
        ctx.emitted(),
        &[vec![
            Value::String("SUMMARY_TEXT".into()),
            Value::String("cols=2 [SPEC:VARCHAR(2000) UTF8,PARAM_COUNT:VARCHAR(2000) UTF8]".into()),
        ]],
        "the worker reports the column count and each declared name and type it read at runtime"
    );
}

/// `EXPORT` discards the generated `SELECT`'s result, so the error channel is
/// the only path the observed specification has back to the client.
#[test]
fn export_worker_returns_its_summary_as_an_error() {
    let mut ctx = TestContext::scalar(vec![Value::String("SUMMARY_TEXT".into())]);

    let err = export_worker(&mut ctx).unwrap_err();

    assert!(
        err.to_string().contains("EXPORT_SPEC SUMMARY_TEXT"),
        "the error text must carry the summary the hook passed: {err}"
    );
}
