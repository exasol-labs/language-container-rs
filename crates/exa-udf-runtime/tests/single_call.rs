//! Single-call dispatch tests against a mock database.
//!
//! Binds a ZMQ `REP` socket and replays the single-call wire protocol while the
//! real [`Runtime`] drives `libsingle_call_fixture.so`. The fixture wires the
//! `default_output_columns` and `virtual_schema_adapter_call` hooks but leaves
//! the import/export SQL hooks `None`, so the runtime must reply `MT_RETURN`
//! for the former and `MT_UNDEFINED_CALL` for the latter.

use exa_proto::{
    ExascriptInfo, ExascriptMetadata, ExascriptResponse, ExascriptSingleCallRep, IterType,
    MessageType, SingleCallFunctionId,
};
use exa_udf_runtime::Runtime;
use prost::Message;
use std::path::PathBuf;

fn fixture_so_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target/debug/libsingle_call_fixture.so");
    p
}

fn response(mt: MessageType, conn: u64) -> ExascriptResponse {
    ExascriptResponse {
        r#type: mt as i32,
        connection_id: conn,
        ..Default::default()
    }
}

fn recv_req(sock: &zmq::Socket) -> exa_proto::ExascriptRequest {
    let bytes = sock.recv_bytes(0).unwrap();
    exa_proto::ExascriptRequest::decode(bytes.as_slice()).unwrap()
}

fn send_resp(sock: &zmq::Socket, resp: &ExascriptResponse) {
    sock.send(resp.encode_to_vec(), 0).unwrap();
}

fn call_response(
    conn: u64,
    fn_id: SingleCallFunctionId,
    json_arg: Option<&str>,
) -> ExascriptResponse {
    let mut resp = response(MessageType::MtCall, conn);
    resp.call = Some(ExascriptSingleCallRep {
        r#fn: fn_id as i32,
        json_arg: json_arg.map(|s| s.to_string()),
        import_specification: None,
        export_specification: None,
    });
    resp
}

/// Drive the handshake (MT_CLIENT -> MT_INFO -> MT_META) in single-call mode and
/// return the bound server socket plus the connection id so each test can
/// continue replaying the call sequence.
fn handshake(server: &zmq::Socket, conn_id: u64, source: &str) {
    let req = recv_req(server);
    assert_eq!(req.r#type, MessageType::MtClient as i32);
    let mut info = response(MessageType::MtInfo, conn_id);
    info.info = Some(ExascriptInfo {
        source_code: source.to_string(),
        script_name: "SINGLE_CALL_UDF".into(),
        ..Default::default()
    });
    send_resp(server, &info);

    let req = recv_req(server);
    assert_eq!(req.r#type, MessageType::MtMeta as i32);
    let mut meta = response(MessageType::MtMeta, conn_id);
    meta.meta = Some(ExascriptMetadata {
        input_iter_type: IterType::PbExactlyOnce as i32,
        output_iter_type: IterType::PbExactlyOnce as i32,
        input_columns: vec![],
        output_columns: vec![],
        single_call_mode: true,
    });
    send_resp(server, &meta);
}

fn spawn_runtime(
    endpoint: String,
) -> std::thread::JoinHandle<Result<(), exa_udf_runtime::RuntimeError>> {
    std::thread::spawn(move || Runtime::new(endpoint, "test-client".into()).run())
}

fn endpoint_for(tag: &str) -> String {
    format!("ipc:///tmp/exa-mock-sc-{}-{}.ipc", tag, std::process::id())
}

#[test]
fn dispatch_invokes_default_output_columns() {
    let so = fixture_so_path();
    assert!(
        so.exists(),
        "build libsingle_call_fixture.so first: {:?}",
        so
    );
    let conn_id = 7u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("doc");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    // MT_RUN -> MT_CALL(SC_FN_DEFAULT_OUTPUT_COLUMNS)
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(
        &server,
        &call_response(
            conn_id,
            SingleCallFunctionId::ScFnDefaultOutputColumns,
            None,
        ),
    );

    // Runtime replies MT_RETURN with the hook's JSON result.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtReturn as i32,
        "expected MT_RETURN"
    );
    let result = req.call_result.expect("call_result").result;
    assert_eq!(result, r#"[{"name":"c0","type":"Int64"}]"#);
    // End the session.
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

#[test]
fn dispatch_invokes_virtual_schema_adapter_call() {
    let so = fixture_so_path();
    assert!(
        so.exists(),
        "build libsingle_call_fixture.so first: {:?}",
        so
    );
    let conn_id = 11u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("vsa");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(
        &server,
        &call_response(
            conn_id,
            SingleCallFunctionId::ScFnVirtualSchemaAdapterCall,
            Some("{}"),
        ),
    );

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtReturn as i32,
        "expected MT_RETURN"
    );
    let result = req.call_result.expect("call_result").result;
    assert_eq!(result, r#"{"echo":{}}"#, "VS adapter echoes the json_arg");
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

#[test]
fn unimplemented_hook_replies_undefined_call() {
    let so = fixture_so_path();
    assert!(
        so.exists(),
        "build libsingle_call_fixture.so first: {:?}",
        so
    );
    let conn_id = 13u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("undef");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    // The fixture leaves generate_sql_for_export_spec as None.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(
        &server,
        &call_response(
            conn_id,
            SingleCallFunctionId::ScFnGenerateSqlForExportSpec,
            Some("{}"),
        ),
    );

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtUndefinedCall as i32,
        "expected MT_UNDEFINED_CALL for an unregistered hook"
    );
    let undef = req.undefined_call.expect("undefined_call");
    assert_eq!(undef.remote_fn, "SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC");
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

/// The DB acknowledges the container's MT_RETURN with MT_RETURN (16), not
/// MT_CLEANUP (11).  The runtime must then close the run with MT_DONE, get
/// MT_CLEANUP, and finish cleanly — mirroring the canonical C++ single-call
/// loop (`send_run` -> `send_return` -> `send_done` -> `send_finished`).
#[test]
fn mt_return_ack_terminates_session() {
    let so = fixture_so_path();
    assert!(
        so.exists(),
        "build libsingle_call_fixture.so first: {:?}",
        so
    );
    let conn_id = 19u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("ret-ack");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(
        &server,
        &call_response(
            conn_id,
            SingleCallFunctionId::ScFnVirtualSchemaAdapterCall,
            Some("{}"),
        ),
    );

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtReturn as i32,
        "expected MT_RETURN"
    );
    // ACK the container's MT_RETURN with MT_RETURN (not MT_CLEANUP).
    send_resp(&server, &response(MessageType::MtReturn, conn_id));

    // After the MT_RETURN ack, the container closes the run with MT_DONE.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtDone as i32,
        "expected MT_DONE after MT_RETURN ack"
    );
    // The DB ends the session with MT_CLEANUP.
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

#[test]
fn single_call_mode_routes_to_dispatcher() {
    // A bare cleanup right after MT_RUN must end the single-call session
    // cleanly, proving meta.single_call_mode routed to the single-call loop
    // (the scalar loop would instead try to pull input with MT_NEXT).
    let so = fixture_so_path();
    assert!(
        so.exists(),
        "build libsingle_call_fixture.so first: {:?}",
        so
    );
    let conn_id = 17u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("route");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtRun as i32,
        "single-call loop opens with MT_RUN, not MT_NEXT"
    );
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}
