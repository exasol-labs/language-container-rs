use super::*;
use crate::messages::{HostAction, HostEvent};
use crate::meta::{ColumnMeta, ExaType, IterType};
use exa_proto::exascript_metadata::ColumnDefinition;
use exa_proto::{
    ColumnType, ExascriptClose, ExascriptInfo, ExascriptMetadata, ExascriptNextDataRep,
    ExascriptPing, ExascriptResponse, ExascriptTableData, IterType as PbIterType, MessageType,
};

fn response(mt: MessageType) -> ExascriptResponse {
    ExascriptResponse {
        r#type: mt as i32,
        connection_id: 7,
        ..Default::default()
    }
}

fn info() -> ExascriptInfo {
    ExascriptInfo {
        database_name: "db".into(),
        database_version: "1".into(),
        script_name: "my_udf".into(),
        source_code: "fn main(){}".into(),
        session_id: 99,
        statement_id: 1,
        node_count: 3,
        node_id: 2,
        vm_id: 0,
        maximal_memory_limit: 0,
        meta_info: None,
        script_schema: "s".into(),
        current_user: None,
        current_schema: None,
        scope_user: None,
    }
}

fn column(name: &str, ct: ColumnType) -> ColumnDefinition {
    ColumnDefinition {
        name: name.into(),
        r#type: Some(ct as i32),
        type_name: "T".into(),
        size: None,
        precision: None,
        scale: None,
    }
}

fn scalar_meta() -> ExascriptMetadata {
    ExascriptMetadata {
        input_iter_type: PbIterType::PbExactlyOnce as i32,
        output_iter_type: PbIterType::PbExactlyOnce as i32,
        input_columns: vec![column("x", ColumnType::PbInt64)],
        output_columns: vec![column("y", ColumnType::PbInt64)],
        single_call_mode: false,
    }
}

fn table(rows: u64) -> ExascriptTableData {
    ExascriptTableData {
        rows,
        rows_in_group: 0,
        ..Default::default()
    }
}

/// Drives the handshake (MT_INFO then MT_META) and returns the resulting meta.
fn run_handshake(proto: &mut Protocol) -> UdfMeta {
    let mut info_resp = response(MessageType::MtInfo);
    info_resp.info = Some(info());
    let (ev, act) = proto.step(info_resp).unwrap();
    assert!(matches!(ev, HostEvent::Pending));
    assert!(matches!(act, Some(HostAction::MetaRequest)));

    let mut meta_resp = response(MessageType::MtMeta);
    meta_resp.meta = Some(scalar_meta());
    let (ev, _) = proto.step(meta_resp).unwrap();
    match ev {
        HostEvent::Meta(m) => m,
        _ => panic!("expected Meta event"),
    }
}

#[test]
fn meta_maps_all_pb_types() {
    let cases = [
        (ColumnType::PbUnsupported, ExaType::Unsupported),
        (ColumnType::PbDouble, ExaType::Double),
        (ColumnType::PbInt32, ExaType::Int32),
        (ColumnType::PbInt64, ExaType::Int64),
        (ColumnType::PbNumeric, ExaType::Numeric),
        (ColumnType::PbTimestamp, ExaType::Timestamp),
        (ColumnType::PbDate, ExaType::Date),
        (ColumnType::PbString, ExaType::String),
        (ColumnType::PbBoolean, ExaType::Boolean),
    ];
    for (pb, expected) in cases {
        let cm = ColumnMeta::from_pb(&column("c", pb));
        assert_eq!(cm.typ, expected, "mapping failed for {pb:?}");
    }
}

#[test]
fn handshake_emits_info_then_meta() {
    let mut proto = Protocol::new();
    let meta = run_handshake(&mut proto);

    assert_eq!(meta.source_code, "fn main(){}");
    assert_eq!(meta.script_name, "my_udf");
    assert_eq!(meta.session_id, 99);
    assert_eq!(meta.node_id, 2);
    assert_eq!(meta.node_count, 3);
    assert_eq!(meta.input_iter, IterType::ExactlyOnce);
    assert_eq!(meta.input_columns.len(), 1);
    assert_eq!(meta.input_columns[0].typ, ExaType::Int64);
    assert_eq!(*proto.phase(), Phase::Run);
    assert_eq!(proto.connection_id(), 7);
}

#[test]
fn scalar_loop_next_emit_done() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto);

    let (ev, _) = proto.step(response(MessageType::MtRun)).unwrap();
    assert!(matches!(ev, HostEvent::Run));

    let mut next_resp = response(MessageType::MtNext);
    next_resp.next = Some(ExascriptNextDataRep { table: table(1) });
    let (ev, _) = proto.step(next_resp).unwrap();
    match ev {
        HostEvent::NextData(t) => assert_eq!(t.rows, 1),
        _ => panic!("expected NextData"),
    }

    let (ev, _) = proto.step(response(MessageType::MtDone)).unwrap();
    assert!(matches!(ev, HostEvent::Done));
    // MT_DONE ends input for the run but keeps the protocol in the run phase;
    // only MT_CLEANUP advances to cleanup.
    assert_eq!(*proto.phase(), Phase::Run);
}

#[test]
fn set_loop_multiple_batches() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto);
    proto.step(response(MessageType::MtRun)).unwrap();

    for rows in [3u64, 5u64] {
        let mut next_resp = response(MessageType::MtNext);
        next_resp.next = Some(ExascriptNextDataRep { table: table(rows) });
        let (ev, _) = proto.step(next_resp).unwrap();
        match ev {
            HostEvent::NextData(t) => assert_eq!(t.rows, rows),
            _ => panic!("expected NextData"),
        }
    }

    let (ev, _) = proto.step(response(MessageType::MtDone)).unwrap();
    assert!(matches!(ev, HostEvent::Done));
}

#[test]
fn close_sequence_cleanup_finished_close() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto);
    proto.step(response(MessageType::MtRun)).unwrap();
    proto.step(response(MessageType::MtDone)).unwrap();

    let (ev, _) = proto.step(response(MessageType::MtCleanup)).unwrap();
    assert!(matches!(ev, HostEvent::Cleanup));
    assert_eq!(*proto.phase(), Phase::Cleanup);

    let (ev, _) = proto.step(response(MessageType::MtFinished)).unwrap();
    assert!(matches!(ev, HostEvent::Finished));
    assert_eq!(*proto.phase(), Phase::Done);

    let mut close_resp = response(MessageType::MtClose);
    close_resp.close = Some(ExascriptClose {
        exception_message: Some("bye".into()),
    });
    let (ev, _) = proto.step(close_resp).unwrap();
    match ev {
        HostEvent::Close(Some(msg)) => assert_eq!(msg, "bye"),
        _ => panic!("expected Close with message"),
    }
}

#[test]
fn ping_pong_echoes() {
    let mut proto = Protocol::new();
    let mut ping_resp = response(MessageType::MtPingPong);
    ping_resp.ping = Some(ExascriptPing {
        meta_info: "hello".into(),
    });
    let (ev, act) = proto.step(ping_resp).unwrap();
    match ev {
        HostEvent::Ping(m) => assert_eq!(m, "hello"),
        _ => panic!("expected Ping"),
    }
    match act {
        Some(HostAction::PingReply(m)) => assert_eq!(m, "hello"),
        _ => panic!("expected PingReply"),
    }
}

#[test]
fn reset_restarts_iteration() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto);
    proto.step(response(MessageType::MtRun)).unwrap();

    let (ev, _) = proto.step(response(MessageType::MtReset)).unwrap();
    assert!(matches!(ev, HostEvent::Reset));
    assert_eq!(*proto.phase(), Phase::Run);
}

#[test]
fn try_again_no_phase_advance() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto);
    proto.step(response(MessageType::MtRun)).unwrap();

    let (ev, _) = proto.step(response(MessageType::MtTryAgain)).unwrap();
    assert!(matches!(ev, HostEvent::TryAgain));
    assert_eq!(*proto.phase(), Phase::Run);
}

#[test]
fn unexpected_message_is_error() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto);

    let err = proto.step(response(MessageType::MtClient)).unwrap_err();
    match err {
        ProtocolError::UnexpectedMessage(ty, state) => {
            assert_eq!(ty, MessageType::MtClient as i32);
            assert_eq!(state, "Run");
        }
        other => panic!("expected UnexpectedMessage, got {other:?}"),
    }
}

#[test]
fn meta_before_info_is_error() {
    let mut proto = Protocol::new();
    let mut meta_resp = response(MessageType::MtMeta);
    meta_resp.meta = Some(scalar_meta());
    let err = proto.step(meta_resp).unwrap_err();
    assert!(matches!(err, ProtocolError::Protocol(_)));
}

#[test]
fn error_close_path_prefix() {
    let proto = Protocol::new();
    let req = proto.error_close_request(1234, "oops");
    let msg = req.close.unwrap().exception_message.unwrap();
    assert!(
        msg.starts_with("F-UDF-CL-RUST-1234:"),
        "unexpected message: {msg}"
    );
    assert_eq!(req.r#type, MessageType::MtClose as i32);
}

#[test]
fn meta_request_is_bare_envelope() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto);
    let req = proto.meta_request();
    assert_eq!(req.r#type, MessageType::MtMeta as i32);
    assert_eq!(req.connection_id, 7);
    assert!(req.client.is_none());
}

#[test]
fn udf_meta_to_pb_round_trips_columns() {
    let mut proto = Protocol::new();
    let meta = run_handshake(&mut proto);
    let pb = meta.to_pb();
    assert_eq!(pb.input_columns.len(), 1);
    assert_eq!(pb.input_columns[0].r#type(), ColumnType::PbInt64);
    assert_eq!(pb.input_iter_type(), PbIterType::PbExactlyOnce);
}

#[test]
fn import_connection_request_sets_type_and_kind() {
    use exa_proto::{ImportType, MessageType};
    let mut proto = Protocol::new();
    // Advance past handshake so connection_id is set.
    run_handshake(&mut proto);

    let req = proto.import_connection_request("");
    assert_eq!(req.r#type, MessageType::MtImport as i32);
    let import = req.import.expect("import field must be set");
    assert_eq!(import.script_name, "");
    assert_eq!(
        import.kind,
        Some(ImportType::PbImportConnectionInformation as i32)
    );
}

#[test]
fn import_connection_request_passes_script_name() {
    use exa_proto::MessageType;
    let mut proto = Protocol::new();
    run_handshake(&mut proto);

    let req = proto.import_connection_request("MY_SCRIPT");
    assert_eq!(req.r#type, MessageType::MtImport as i32);
    let import = req.import.expect("import field must be set");
    assert_eq!(import.script_name, "MY_SCRIPT");
}
