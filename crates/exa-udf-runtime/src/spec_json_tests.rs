use super::*;
use exa_proto::exascript_metadata::ColumnDefinition;
use exa_proto::{ColumnType, ConnectionInformationRep, KeyValuePair};

fn pair(key: &str, value: &str) -> KeyValuePair {
    KeyValuePair {
        key: key.into(),
        value: value.into(),
    }
}

fn credentials() -> ConnectionInformationRep {
    ConnectionInformationRep {
        kind: "password".into(),
        address: "10.0.0.5:8563".into(),
        user: "sys".into(),
        password: "exasol".into(),
    }
}

fn parse(json: &str) -> serde_json::Value {
    serde_json::from_str(json).expect("the serializer must emit valid JSON")
}

#[test]
fn spec_json_mirrors_every_proto_field() {
    let import = ImportSpecificationRep {
        is_subselect: true,
        connection_information: Some(credentials()),
        connection_name: None,
        subselect_column_specification: vec![
            ColumnDefinition {
                name: "AMOUNT".into(),
                r#type: Some(ColumnType::PbDouble as i32),
                type_name: "DOUBLE".into(),
                size: None,
                precision: None,
                scale: None,
            },
            ColumnDefinition {
                name: "LABEL".into(),
                r#type: Some(ColumnType::PbString as i32),
                type_name: "VARCHAR(20) UTF8".into(),
                size: Some(20),
                precision: Some(18),
                scale: Some(2),
            },
        ],
        parameters: vec![pair("FILE", "a.csv"), pair("FILE", "b.csv")],
    };

    assert_eq!(
        parse(&serialize_import(&import)),
        serde_json::json!({
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
                    "type": "PB_STRING",
                    "type_name": "VARCHAR(20) UTF8",
                    "size": 20,
                    "precision": 18,
                    "scale": 2
                }
            ],
            "parameters": [
                { "key": "FILE", "value": "a.csv" },
                { "key": "FILE", "value": "b.csv" }
            ]
        })
    );

    let export = ExportSpecificationRep {
        has_truncate: true,
        has_replace: false,
        created_by: Some("CREATE TABLE T (K DECIMAL(18,0))".into()),
        source_column_names: vec!["T.K".into(), "T.LABEL".into()],
        connection_information: None,
        connection_name: Some("MY_CONN".into()),
        parameters: vec![pair("SHARDS", "4")],
    };

    assert_eq!(
        parse(&serialize_export(&export)),
        serde_json::json!({
            "has_truncate": true,
            "has_replace": false,
            "created_by": "CREATE TABLE T (K DECIMAL(18,0))",
            "source_column_names": ["T.K", "T.LABEL"],
            "connection_information": null,
            "connection_name": "MY_CONN",
            "parameters": [{ "key": "SHARDS", "value": "4" }]
        })
    );
}

/// The key set never varies with what the database populated, so a consumer
/// parses one shape instead of branching on which keys arrived.
#[test]
fn an_unpopulated_spec_still_carries_every_key() {
    let import = parse(&serialize_import(&ImportSpecificationRep {
        is_subselect: false,
        connection_information: None,
        connection_name: None,
        subselect_column_specification: Vec::new(),
        parameters: Vec::new(),
    }));

    assert_eq!(
        import,
        serde_json::json!({
            "is_subselect": false,
            "connection_information": null,
            "connection_name": null,
            "subselect_column_specification": [],
            "parameters": []
        })
    );

    let export = parse(&serialize_export(&ExportSpecificationRep {
        has_truncate: false,
        has_replace: true,
        created_by: None,
        source_column_names: Vec::new(),
        connection_information: None,
        connection_name: None,
        parameters: Vec::new(),
    }));

    assert_eq!(
        export,
        serde_json::json!({
            "has_truncate": false,
            "has_replace": true,
            "created_by": null,
            "source_column_names": [],
            "connection_information": null,
            "connection_name": null,
            "parameters": []
        })
    );
}

/// A `column_type` value this build's proto does not name has no variant name
/// to report. It degrades to `null` rather than to the raw number, so a payload
/// from a database that widened the enum still parses as a whole; `type_name`
/// carries the database's own rendering either way.
#[test]
fn an_unnamed_column_type_degrades_to_null() {
    let import = parse(&serialize_import(&ImportSpecificationRep {
        is_subselect: false,
        connection_information: None,
        connection_name: None,
        subselect_column_specification: vec![ColumnDefinition {
            name: "FUTURE".into(),
            r#type: Some(4242),
            type_name: "GEOMETRY".into(),
            size: None,
            precision: None,
            scale: None,
        }],
        parameters: Vec::new(),
    }));

    let column = &import["subselect_column_specification"][0];
    assert_eq!(column["type"], serde_json::Value::Null);
    assert_eq!(column["type_name"], "GEOMETRY");
}
