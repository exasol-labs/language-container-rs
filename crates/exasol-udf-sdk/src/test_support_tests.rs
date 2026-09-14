use super::{DefaultsCtx, EmitPolicy, NextPolicy, TestContext};
use crate::connect_back::ConnectionObject;
use crate::context::UdfContext;
use crate::error::UdfError;
use crate::value::{ColumnInfo, ExaType, Value};

#[test]
fn test_context_covers_scalar_set_emit_and_return_paths() {
    let mut scalar = TestContext::scalar(vec![Value::Int64(7), Value::String("a".into())]);
    assert_eq!(scalar.num_columns(), 2);
    assert_eq!(scalar.get(0).unwrap(), &Value::Int64(7));
    assert_eq!(scalar.get_string(1).unwrap(), Some("a"));
    assert!(!scalar.next().unwrap(), "scalar input is a single row");
    scalar.emit(vec![Value::Int64(1)]).unwrap();
    scalar.set_return(Some(Value::Int64(42))).unwrap();
    assert_eq!(scalar.emitted(), &[vec![Value::Int64(1)]]);
    assert_eq!(scalar.captured_return(), Some(&Some(Value::Int64(42))));

    let mut group = TestContext::set(vec![vec![Value::Int64(1)], vec![Value::Int64(2)]]);
    assert_eq!(group.num_columns(), 1);
    let mut seen = Vec::new();
    while group.next().unwrap() {
        seen.push(group.get_i64(0).unwrap());
    }
    assert_eq!(seen, vec![Some(1), Some(2)]);
    assert!(
        !group.next().unwrap(),
        "next stays false past the group boundary"
    );
}

#[test]
fn captured_return_separates_never_called_from_called_with_none() {
    let untouched = TestContext::scalar(vec![]);
    assert_eq!(untouched.captured_return(), None);

    let mut nulled = TestContext::scalar(vec![]);
    nulled.set_return(None).unwrap();
    assert_eq!(nulled.captured_return(), Some(&None));
}

#[test]
fn emit_policy_rejects_every_call_with_the_supplied_error() {
    let mut ctx = TestContext::scalar(vec![Value::Int64(1)]).with_emit_policy(EmitPolicy::Reject(
        UdfError::Unimplemented("emit is banned in RETURNS output".into()),
    ));

    for _ in 0..2 {
        let err = ctx.emit(vec![Value::Int64(1)]).unwrap_err();
        assert!(matches!(err, UdfError::Unimplemented(msg) if msg.contains("RETURNS")));
    }
    assert!(
        ctx.emitted().is_empty(),
        "a rejected emit records no output row"
    );
}

#[test]
fn next_policy_rejects_every_call_with_the_supplied_error() {
    let mut ctx =
        TestContext::set(vec![vec![Value::Int64(1)]]).with_next_policy(NextPolicy::Reject(
            UdfError::User("next() is not allowed in scalar context".into()),
        ));

    for _ in 0..2 {
        let err = ctx.next().unwrap_err();
        assert!(matches!(err, UdfError::User(msg) if msg.contains("scalar")));
    }
}

#[test]
fn metadata_defaults_match_the_trait_defaults() {
    let ctx = TestContext::scalar(vec![]);
    let defaults = DefaultsCtx;

    assert_eq!(ctx.memory_limit(), defaults.memory_limit());
    assert_eq!(ctx.session_id(), defaults.session_id());
    assert_eq!(ctx.statement_id(), defaults.statement_id());
    assert_eq!(ctx.node_id(), defaults.node_id());
    assert_eq!(ctx.node_count(), defaults.node_count());
    assert_eq!(ctx.vm_id(), defaults.vm_id());
    assert_eq!(ctx.database_name(), defaults.database_name());
    assert_eq!(ctx.database_version(), defaults.database_version());
    assert_eq!(ctx.script_name(), defaults.script_name());
    assert_eq!(ctx.script_schema(), defaults.script_schema());
    assert_eq!(ctx.current_user(), defaults.current_user());
    assert_eq!(ctx.current_schema(), defaults.current_schema());
    assert_eq!(ctx.scope_user(), defaults.scope_user());
    assert_eq!(ctx.debug_level(), defaults.debug_level());

    assert!(ctx.cluster_ip().is_err(), "cluster_ip unset by default");
    assert!(
        ctx.connection("ANY").is_err(),
        "connection unset by default"
    );
}

#[test]
fn column_metadata_is_unset_until_supplied() {
    let bare = TestContext::scalar(vec![Value::Int64(1)]);
    assert!(bare.input_column(0).is_err(), "no schema supplied");
    assert_eq!(
        bare.output_column_count(),
        DefaultsCtx.output_column_count()
    );
    assert!(DefaultsCtx.input_column(0).is_err());
    assert!(DefaultsCtx.output_column(0).is_err());

    let column = |name: &str, typ: ExaType| ColumnInfo {
        name: name.into(),
        typ,
        type_name: String::new(),
        size: None,
        precision: None,
        scale: None,
    };
    let ctx = TestContext::scalar(vec![Value::Int64(1)])
        .with_input_columns(vec![column("x", ExaType::Int64)])
        .with_output_columns(vec![column("y", ExaType::Double)]);

    assert_eq!(ctx.input_column(0).unwrap().name, "x");
    assert_eq!(ctx.output_column_count(), 1);
    assert_eq!(ctx.output_column(0).unwrap().typ, ExaType::Double);
    assert!(ctx.output_column(1).is_err());
}

#[test]
fn metadata_setters_override_every_accessor() {
    let ctx = TestContext::scalar(vec![])
        .with_memory_limit(4_000_000)
        .with_session_id(1_700_000_000_000_123)
        .with_statement_id(3)
        .with_node_id(1)
        .with_node_count(4)
        .with_vm_id(99)
        .with_database_name("exadb")
        .with_database_version("8.29.13")
        .with_script_name("handshake_meta")
        .with_script_schema("IT_RUST")
        .with_current_user("SYS")
        .with_current_schema("IT_RUST_OTHER")
        .with_scope_user("IT_VIEW_OWNER")
        .with_debug_level(tracing::Level::TRACE)
        .with_cluster_ip("10.0.0.5")
        .with_connection(
            "MY_CONN",
            ConnectionObject {
                kind: "EXA".into(),
                address: "10.0.0.5:8563".into(),
                user: "sys".into(),
                password: "secret".into(),
            },
        );

    assert_eq!(ctx.memory_limit(), 4_000_000);
    assert_eq!(ctx.session_id(), 1_700_000_000_000_123);
    assert_eq!(ctx.statement_id(), 3);
    assert_eq!(ctx.node_id(), 1);
    assert_eq!(ctx.node_count(), 4);
    assert_eq!(ctx.vm_id(), 99);
    assert_eq!(ctx.database_name(), "exadb");
    assert_eq!(ctx.database_version(), "8.29.13");
    assert_eq!(ctx.script_name(), "handshake_meta");
    assert_eq!(ctx.script_schema(), "IT_RUST");
    assert_eq!(ctx.current_user().as_deref(), Some("SYS"));
    assert_eq!(ctx.current_schema().as_deref(), Some("IT_RUST_OTHER"));
    assert_eq!(ctx.scope_user().as_deref(), Some("IT_VIEW_OWNER"));
    assert_eq!(ctx.debug_level(), tracing::Level::TRACE);
    assert_eq!(ctx.cluster_ip().unwrap(), "10.0.0.5");
    let conn = ctx.connection("MY_CONN").unwrap();
    assert_eq!(conn.address, "10.0.0.5:8563");
    assert_eq!(conn.user, "sys");
}

#[test]
fn connection_lookup_is_case_insensitive() {
    let obj = ConnectionObject {
        kind: "EXA".into(),
        address: "10.0.0.1:8563".into(),
        user: "u".into(),
        password: "p".into(),
    };
    let ctx = TestContext::scalar(vec![]).with_connection("my_conn", obj);
    assert!(ctx.connection("MY_CONN").is_ok());
    assert!(ctx.connection("my_conn").is_ok());
    assert!(ctx.connection("MISSING").is_err());
}

#[test]
fn get_beyond_the_current_row_errors_instead_of_panicking() {
    let ctx = TestContext::scalar(vec![Value::Int64(1)]);
    let err = ctx.get(1).unwrap_err();
    assert!(matches!(err, UdfError::Type(msg) if msg.contains('1')));
}

#[test]
fn get_before_the_first_next_errors_instead_of_panicking() {
    let ctx = TestContext::set(vec![vec![Value::Int64(1)]]);
    let err = ctx.get(0).unwrap_err();
    assert!(matches!(err, UdfError::Type(msg) if msg.contains("next")));
}

#[test]
fn empty_set_group_reports_no_columns_and_no_rows() {
    let mut ctx = TestContext::set(Vec::new());
    assert_eq!(ctx.num_columns(), 0);
    assert!(!ctx.next().unwrap());
    assert!(ctx.get(0).is_err());
}

#[test]
fn defaults_ctx_overrides_no_provided_method() {
    let mut ctx = DefaultsCtx;

    assert_eq!(ctx.memory_limit(), 0);
    assert_eq!(ctx.session_id(), 0);
    assert_eq!(ctx.current_user(), None);
    assert_eq!(ctx.debug_level(), tracing::Level::INFO);
    assert!(matches!(
        ctx.set_return(Some(Value::Int64(1))).unwrap_err(),
        UdfError::Unimplemented(_)
    ));
}

#[test]
fn defaults_ctx_reports_no_columns_and_accepts_emit() {
    let mut ctx = DefaultsCtx;

    assert_eq!(ctx.num_columns(), 0);
    assert!(matches!(ctx.get(0).unwrap_err(), UdfError::Type(_)));
    assert!(ctx.emit(vec![Value::Int64(1)]).is_ok());
    assert!(!ctx.next().unwrap());
}
