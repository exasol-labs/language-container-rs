use exa_proto::{
    ConnectionInformationRep, ExascriptImportRep, ExascriptInfo, ExascriptMetadata,
    ExascriptResponse, ExascriptSingleCallRep, IterType as PbIterType, MessageType,
    SingleCallFunctionId,
};
use exa_zmq_protocol::{HostEvent, Protocol};

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

fn single_call_meta() -> ExascriptMetadata {
    ExascriptMetadata {
        input_iter_type: PbIterType::PbExactlyOnce as i32,
        output_iter_type: PbIterType::PbExactlyOnce as i32,
        input_columns: vec![],
        output_columns: vec![],
        single_call_mode: true,
    }
}

fn scalar_meta() -> ExascriptMetadata {
    ExascriptMetadata {
        input_iter_type: PbIterType::PbExactlyOnce as i32,
        output_iter_type: PbIterType::PbExactlyOnce as i32,
        input_columns: vec![],
        output_columns: vec![],
        single_call_mode: false,
    }
}

/// Drives the handshake with the given metadata, returns (proto, meta).
fn run_handshake(proto: &mut Protocol, meta: ExascriptMetadata) -> exa_zmq_protocol::UdfMeta {
    let mut info_resp = response(MessageType::MtInfo);
    info_resp.info = Some(info());
    proto.step(info_resp).unwrap();

    let mut meta_resp = response(MessageType::MtMeta);
    meta_resp.meta = Some(meta);
    let (ev, _) = proto.step(meta_resp).unwrap();
    match ev {
        HostEvent::Meta(m) => m,
        _ => panic!("expected Meta event"),
    }
}

// --- Task 1.1 + 1.2: MT_CALL decoding ---

#[test]
fn mt_call_emits_single_call_event() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto, single_call_meta());

    let mut call_resp = response(MessageType::MtCall);
    call_resp.call = Some(ExascriptSingleCallRep {
        r#fn: SingleCallFunctionId::ScFnVirtualSchemaAdapterCall as i32,
        json_arg: Some(r#"{"key":"val"}"#.into()),
        import_specification: None,
        export_specification: None,
    });

    let (ev, _) = proto.step(call_resp).unwrap();
    match ev {
        HostEvent::SingleCall {
            fn_id, json_arg, ..
        } => {
            assert_eq!(fn_id, SingleCallFunctionId::ScFnVirtualSchemaAdapterCall);
            assert_eq!(json_arg.as_deref(), Some(r#"{"key":"val"}"#));
        }
        _ => panic!("expected SingleCall event, got {ev:?}"),
    }
}

#[test]
fn single_call_return_serializes_mt_return() {
    let proto = Protocol::new();
    let req = proto.return_request("my_result".into());
    assert_eq!(req.r#type, MessageType::MtReturn as i32);
    let ret = req.call_result.expect("call_result must be set");
    assert_eq!(ret.result, "my_result");
}

#[test]
fn undefined_call_serializes_mt_undefined_call() {
    let proto = Protocol::new();
    let req = proto.undefined_call_request("some_remote_fn");
    assert_eq!(req.r#type, MessageType::MtUndefinedCall as i32);
    let uc = req.undefined_call.expect("undefined_call must be set");
    assert_eq!(uc.remote_fn, "some_remote_fn");
}

// --- Task 1.3: connection information ---

#[test]
fn conn_info_is_parsed_from_import_response() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto, single_call_meta());

    // Simulate the DB responding to MT_IMPORT with connection info
    let mut import_resp = response(MessageType::MtImport);
    import_resp.import = Some(ExascriptImportRep {
        source_code: None,
        exception_message: None,
        connection_information: Some(ConnectionInformationRep {
            kind: "PASSWORD".into(),
            address: "127.0.0.1:8563".into(),
            user: "sys".into(),
            password: "secret".into(),
        }),
    });

    let (ev, _) = proto.step(import_resp).unwrap();
    match ev {
        HostEvent::ConnInfo(ci) => {
            assert_eq!(ci.kind, "PASSWORD");
            assert_eq!(ci.address, "127.0.0.1:8563");
            assert_eq!(ci.user, "sys");
            assert_eq!(ci.password, "secret");
        }
        _ => panic!("expected ConnInfo event, got {ev:?}"),
    }
}

// --- Task 1.4: MT_CALL in non-single-call mode is a protocol error ---

#[test]
fn mt_call_in_non_single_call_mode_is_protocol_error() {
    let mut proto = Protocol::new();
    run_handshake(&mut proto, scalar_meta());

    // MT_CALL should not be valid when single_call_mode=false
    let mut call_resp = response(MessageType::MtCall);
    call_resp.call = Some(ExascriptSingleCallRep {
        r#fn: SingleCallFunctionId::ScFnVirtualSchemaAdapterCall as i32,
        json_arg: None,
        import_specification: None,
        export_specification: None,
    });

    let err = proto.step(call_resp).unwrap_err();
    match err {
        exa_zmq_protocol::ProtocolError::UnexpectedMessage(ty, _state) => {
            assert_eq!(ty, MessageType::MtCall as i32);
        }
        other => panic!("expected UnexpectedMessage, got {other:?}"),
    }
}
