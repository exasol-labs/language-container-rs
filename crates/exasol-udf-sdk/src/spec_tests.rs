use super::*;

/// The `json_spec` shape the runtime pins: every proto field present under its
/// proto name, `repeated` as an array, absent `optional` as `null`.
const IMPORT_JSON: &str = r#"{
  "is_subselect": true,
  "connection_information": {
    "kind": "password",
    "address": "10.0.0.5:8563",
    "user": "sys",
    "password": "exasol"
  },
  "connection_name": null,
  "subselect_column_specification": [
    {
      "name": "AMOUNT",
      "type": "PB_DOUBLE",
      "type_name": "DOUBLE",
      "size": null,
      "precision": null,
      "scale": null
    },
    {
      "name": "LABEL",
      "type": "PB_CHAR",
      "type_name": "VARCHAR(20) UTF8",
      "size": 20,
      "precision": null,
      "scale": null
    }
  ],
  "parameters": [
    { "key": "FILE", "value": "a.csv" },
    { "key": "FILE", "value": "b.csv" }
  ]
}"#;

const EXPORT_JSON: &str = r#"{
  "has_truncate": true,
  "has_replace": false,
  "created_by": "CREATE TABLE T (K DECIMAL(18,0))",
  "source_column_names": ["T.K", "T.LABEL"],
  "connection_information": null,
  "connection_name": "MY_CONN",
  "parameters": [{ "key": "SHARDS", "value": "4" }]
}"#;

#[test]
fn import_and_export_spec_parse_the_pinned_json_shape() {
    let import = ImportSpec::from_json(IMPORT_JSON).unwrap();
    assert!(import.is_subselect);
    assert_eq!(import.connection_name, None);
    let conn = import.connection_information.unwrap();
    assert_eq!(conn.kind, "password");
    assert_eq!(conn.address, "10.0.0.5:8563");
    assert_eq!(conn.user, "sys");
    assert_eq!(conn.password, "exasol");

    let columns = &import.subselect_column_specification;
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].r#type.as_deref(), Some("PB_DOUBLE"));
    assert_eq!(columns[0].type_name, "DOUBLE");
    assert_eq!(columns[0].size, None);
    assert_eq!(columns[1].name, "LABEL");
    assert_eq!(columns[1].size, Some(20));

    let params: Vec<(&str, &str)> = import
        .parameters
        .iter()
        .map(|p| (p.key.as_str(), p.value.as_str()))
        .collect();
    assert_eq!(params, vec![("FILE", "a.csv"), ("FILE", "b.csv")]);

    let export = ExportSpec::from_json(EXPORT_JSON).unwrap();
    assert!(export.has_truncate);
    assert!(!export.has_replace);
    assert_eq!(
        export.created_by.as_deref(),
        Some("CREATE TABLE T (K DECIMAL(18,0))")
    );
    assert_eq!(export.source_column_names, vec!["T.K", "T.LABEL"]);
    assert!(export.connection_information.is_none());
    assert_eq!(export.connection_name.as_deref(), Some("MY_CONN"));
    assert_eq!(export.parameters.len(), 1);
    assert_eq!(export.parameters[0].key, "SHARDS");
}

#[test]
fn spec_parsers_ignore_an_unknown_json_field() {
    let import = IMPORT_JSON.replacen('{', r#"{ "widened_later": 7,"#, 1);
    assert!(ImportSpec::from_json(&import).unwrap().is_subselect);

    let export = EXPORT_JSON.replacen('{', r#"{ "widened_later": [1, 2],"#, 1);
    assert!(ExportSpec::from_json(&export).unwrap().has_truncate);
}

#[test]
fn spec_parsers_report_an_unreadable_payload_as_a_type_error() {
    let err = ImportSpec::from_json("{").unwrap_err();
    assert!(
        matches!(&err, UdfError::Type(msg) if msg.contains("IMPORT")),
        "{err:?}"
    );

    let err = ExportSpec::from_json(r#"{"has_truncate": "yes"}"#).unwrap_err();
    assert!(
        matches!(&err, UdfError::Type(msg) if msg.contains("EXPORT")),
        "{err:?}"
    );
}
