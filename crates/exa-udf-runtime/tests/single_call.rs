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

mod common;
use common::fixture_so_path;

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
    std::thread::spawn(move || Runtime::new(endpoint, "test-client".into()).run(|_| {}))
}

fn endpoint_for(tag: &str) -> String {
    format!("ipc:///tmp/exa-mock-sc-{}-{}.ipc", tag, std::process::id())
}

#[test]
fn dispatch_invokes_default_output_columns() {
    let so = fixture_so_path("single_call_fixture");
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

/// The `virtual_schema_adapter_call` hook restores the live `UdfContext` off the
/// ABI double-indirection and deliberately fails (rc=1), writing the live
/// handshake metadata into the error out-pointer. The runtime surfaces a hook
/// error by closing the wire with MT_CLOSE (F-UDF-CL-RUST-9001) carrying that
/// text and returning `Err` from `run()`. The mock controls the handshake
/// values (all numerics default to 0, `script_name` = "SINGLE_CALL_UDF"), so we
/// assert the exact echoed metadata rather than a non-zero gate.
#[test]
fn dispatch_surfaces_adapter_hook_error() {
    let so = fixture_so_path("single_call_fixture");
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

    // The hook returns rc=1, so the runtime does not reply MT_RETURN; it closes
    // the wire with an MT_CLOSE carrying the fixture's HANDSHAKE_META error text
    // (prefixed with the F-UDF-CL-RUST-9001 close code).
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "adapter hook error must close the wire with MT_CLOSE"
    );
    let close_msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        close_msg.contains("F-UDF-CL-RUST-9001"),
        "close carries the UDF error close code: {close_msg:?}"
    );
    assert!(
        close_msg.contains(
            "HANDSHAKE_META node_count=0 node_id=0 session_id=0 script_name=SINGLE_CALL_UDF"
        ),
        "close echoes the live handshake metadata read off the ctx pointer: {close_msg:?}"
    );

    // The runtime surfaces the same error to the caller and does not wait for a
    // reply to the close; the client thread ends with Err.
    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("adapter hook error must surface as Err");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains(
            "HANDSHAKE_META node_count=0 node_id=0 session_id=0 script_name=SINGLE_CALL_UDF"
        ),
        "runtime error surfaces the live handshake metadata: {err_msg:?}"
    );
}

#[test]
fn unimplemented_hook_replies_undefined_call() {
    let so = fixture_so_path("single_call_fixture");
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
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 19u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("ret-ack");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    // Use the always-succeeding `default_output_columns` hook so the container
    // produces an MT_RETURN to ack; the VS-adapter hook now deliberately fails
    // (rc=1), which would close the wire instead of returning.
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

/// No other message is valid as the direct answer to MT_RUN in single-call
/// mode: retrying would risk a livelock, so an unrecognized event is a hard
/// error rather than something the client tolerates.
#[test]
fn unexpected_event_in_single_call_mode_is_hard_error() {
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 31u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("unexpected");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtDone, conn_id));

    // The hard error surfaces as the client's own MT_CLOSE, relayed before
    // returning Err; it does not wait for a reply.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "an unexpected event must close the wire with MT_CLOSE"
    );
    let close_msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        close_msg.contains("F-UDF-CL-RUST-9001"),
        "close carries the UDF error close code: {close_msg:?}"
    );
    assert!(
        close_msg.contains("unexpected message in single-call mode"),
        "close names the hard-error policy: {close_msg:?}"
    );

    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("an unexpected event must surface as Err");
    assert!(
        err.to_string()
            .contains("unexpected message in single-call mode"),
        "runtime error names the hard-error policy: {err}"
    );
}

#[test]
fn single_call_mode_routes_to_dispatcher() {
    // A bare cleanup right after MT_RUN must end the single-call session
    // cleanly, proving meta.single_call_mode routed to the single-call loop
    // (the scalar loop would instead try to pull input with MT_NEXT).
    let so = fixture_so_path("single_call_fixture");
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

#[test]
fn close_ack_after_call_reply_ends_session() {
    // run_single_call's reply-ack match Close arm: the DB can ack the
    // container's MT_RETURN with MT_CLOSE instead of SingleCallAck/MT_CLEANUP.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 71u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("closeack");

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
            SingleCallFunctionId::ScFnDefaultOutputColumns,
            None,
        ),
    );

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtReturn as i32,
        "expected MT_RETURN"
    );
    let mut close = response(MessageType::MtClose, conn_id);
    close.close = Some(exa_proto::ExascriptClose {
        exception_message: Some("ack boom".into()),
    });
    send_resp(&server, &close);

    // The runtime relays the DB's close as its own MT_CLOSE before returning.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "a close ack must relay as the runtime's own MT_CLOSE"
    );
    let relayed = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        relayed.contains("ack boom"),
        "relayed close carries the DB's original message, got: {relayed}"
    );

    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("a close ack must surface as Err");
    assert!(
        err.to_string().contains("ack boom"),
        "runtime error carries the DB's message: {err}"
    );
}

#[test]
fn unexpected_ack_after_call_reply_is_hard_error() {
    // run_single_call's reply-ack match wildcard arm: any event other than
    // SingleCallAck/Cleanup/Close as the ack to the container's MT_RETURN is a
    // hard error, like an unexpected event anywhere else in single-call mode.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 73u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("unexpectedack");

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
            SingleCallFunctionId::ScFnDefaultOutputColumns,
            None,
        ),
    );

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtReturn as i32,
        "expected MT_RETURN"
    );
    send_resp(&server, &response(MessageType::MtDone, conn_id));

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "an unexpected ack must close the wire with MT_CLOSE"
    );
    let close_msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        close_msg.contains("unexpected message in single-call mode"),
        "close names the hard-error policy: {close_msg:?}"
    );

    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("an unexpected ack must surface as Err");
    assert!(
        err.to_string()
            .contains("unexpected message in single-call mode"),
        "runtime error names the hard-error policy: {err}"
    );
}

#[test]
fn close_directly_after_run_request_with_no_call_pending() {
    // run_single_call's top-level post-MT_RUN match Close arm: MT_CLOSE can
    // answer MT_RUN directly, before any MT_CALL is issued.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 79u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("closenocall");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    let mut close = response(MessageType::MtClose, conn_id);
    close.close = Some(exa_proto::ExascriptClose {
        exception_message: Some("no call for you".into()),
    });
    send_resp(&server, &close);

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "the runtime relays the DB's close as its own MT_CLOSE"
    );
    let relayed = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        relayed.contains("no call for you"),
        "relayed close carries the DB's original message, got: {relayed}"
    );

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_err(),
        "a close with no call pending must surface as a run error"
    );
}

#[test]
fn done_continues_to_second_call_cycle() {
    // run_single_call's post-MT_DONE match Done arm: answering the container's
    // MT_DONE with MT_DONE (not MT_CLEANUP) continues the session into a
    // second MT_RUN/MT_CALL cycle rather than ending it.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 83u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("secondcycle");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    // First call cycle.
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
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtReturn as i32);
    send_resp(&server, &response(MessageType::MtReturn, conn_id));
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    send_resp(&server, &response(MessageType::MtDone, conn_id));

    // Second call cycle: the DB opens another MT_RUN instead of having ended
    // the session, proving the loop continued.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtRun as i32,
        "MT_DONE answered with MT_DONE must continue the session"
    );
    send_resp(
        &server,
        &call_response(
            conn_id,
            SingleCallFunctionId::ScFnDefaultOutputColumns,
            None,
        ),
    );
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtReturn as i32);
    send_resp(&server, &response(MessageType::MtReturn, conn_id));
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

#[test]
fn close_after_done_request_in_single_call_mode() {
    // run_single_call's post-MT_DONE match Close arm: the DB can end the
    // session with MT_CLOSE as the answer to the container's own MT_DONE.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 89u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("closeafterdone");

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
            SingleCallFunctionId::ScFnDefaultOutputColumns,
            None,
        ),
    );
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtReturn as i32);
    send_resp(&server, &response(MessageType::MtReturn, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    let mut close = response(MessageType::MtClose, conn_id);
    close.close = Some(exa_proto::ExascriptClose {
        exception_message: Some("done teardown boom".into()),
    });
    send_resp(&server, &close);

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "the runtime relays the DB's close as its own MT_CLOSE"
    );
    let relayed = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        relayed.contains("done teardown boom"),
        "relayed close carries the DB's original message, got: {relayed}"
    );

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_err(),
        "a close after MT_DONE must surface as a run error"
    );
}

#[test]
fn unexpected_after_done_request_in_single_call_mode() {
    // run_single_call's post-MT_DONE match wildcard arm: any event other than
    // Done/Cleanup/Close as the answer to the container's MT_DONE is a hard
    // error.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 97u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("unexpectedafterdone");

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
            SingleCallFunctionId::ScFnDefaultOutputColumns,
            None,
        ),
    );
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtReturn as i32);
    send_resp(&server, &response(MessageType::MtReturn, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    send_resp(&server, &response(MessageType::MtRun, conn_id));

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "an unexpected event after MT_DONE must close the wire with MT_CLOSE"
    );
    let close_msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        close_msg.contains("unexpected message in single-call mode"),
        "close names the hard-error policy: {close_msg:?}"
    );

    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("an unexpected event after MT_DONE must surface as Err");
    assert!(
        err.to_string()
            .contains("unexpected message in single-call mode"),
        "runtime error names the hard-error policy: {err}"
    );
}

#[test]
fn import_spec_hook_error_surfaces_as_run_error() {
    // invoke_hook's ScFnGenerateSqlForImportSpec arm and the Some(Err(e)) arm
    // of the result-mapping match: the fixture's hook returns rc=1 with the
    // echoed spec, so the hook error propagates as a run error.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 101u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("importspec");

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
            SingleCallFunctionId::ScFnGenerateSqlForImportSpec,
            Some(r#"{"x":1}"#),
        ),
    );

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "the import-spec hook's error must close the wire with MT_CLOSE"
    );
    let close_msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        close_msg.contains("F-UDF-CL-RUST-9001"),
        "close carries the UDF error close code: {close_msg:?}"
    );
    assert!(
        close_msg.contains(r#"IMPORT_SPEC_HOOK_ERROR arg={"x":1}"#),
        "close echoes the hook's own error text with the spec it received: {close_msg:?}"
    );

    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("the import-spec hook's error must surface as Err");
    assert!(
        err.to_string().contains("IMPORT_SPEC_HOOK_ERROR"),
        "runtime error carries the hook's error text: {err}"
    );
}

#[test]
fn unrecognized_call_fn_id_replies_undefined_call() {
    // invoke_hook's ScFnNil sentinel arm: an MT_CALL naming an unrecognized
    // function id (Protocol::step falls back to ScFnNil) must be treated as an
    // unimplemented hook rather than panic or misroute.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 103u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("unrecognizedfn");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());
    handshake(&server, conn_id, &source);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    let mut resp = response(MessageType::MtCall, conn_id);
    resp.call = Some(ExascriptSingleCallRep {
        r#fn: 9999,
        json_arg: None,
        import_specification: None,
        export_specification: None,
    });
    send_resp(&server, &resp);

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtUndefinedCall as i32,
        "expected MT_UNDEFINED_CALL for an unrecognized fn id"
    );
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

#[test]
fn adapter_hook_success_returns_via_mt_return() {
    // invoke_vs_adapter_call's success arm (Some(Ok(s))): the runtime replies
    // MT_RETURN with the hook's result. Every other adapter test drives the
    // fixture's deliberate-failure default instead.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 107u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("adaptersuccess");

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
            Some("SUCCEED"),
        ),
    );

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtReturn as i32,
        "expected MT_RETURN"
    );
    assert_eq!(
        req.call_result.expect("call_result").result,
        "VS_ADAPTER_OK",
        "the adapter hook's own result must reach MT_RETURN"
    );
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

#[test]
fn vs_adapter_hook_undefined_when_not_registered() {
    // invoke_vs_adapter_call's None arm: an unset virtual_schema_adapter_call
    // must reply MT_UNDEFINED_CALL rather than count as an error.
    let so = fixture_so_path("scalar_double");
    let conn_id = 109u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("vsadapterundef");

    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let client = spawn_runtime(endpoint.clone());

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtClient as i32);
    let mut info = response(MessageType::MtInfo, conn_id);
    info.info = Some(ExascriptInfo {
        source_code: source,
        script_name: "SCALAR_DOUBLE".into(),
        ..Default::default()
    });
    send_resp(&server, &info);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtMeta as i32);
    let mut meta = response(MessageType::MtMeta, conn_id);
    meta.meta = Some(ExascriptMetadata {
        input_iter_type: IterType::PbExactlyOnce as i32,
        output_iter_type: IterType::PbExactlyOnce as i32,
        input_columns: vec![],
        output_columns: vec![],
        single_call_mode: true,
    });
    send_resp(&server, &meta);

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
        MessageType::MtUndefinedCall as i32,
        "expected MT_UNDEFINED_CALL when the hook is unregistered"
    );
    let undef = req.undefined_call.expect("undefined_call");
    assert_eq!(undef.remote_fn, "SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL");
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

/// `wire::conn_requester`'s Close arm: when the DB answers a mid-call
/// `MT_IMPORT` with `MT_CLOSE`, the connect-back error must carry the DB's own
/// exception message rather than the generic "not ConnInfo" text.
#[cfg(feature = "connect-back")]
#[test]
fn adapter_connection_probe_close_preserves_db_exception_message() {
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 114u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("connectionprobeclose");

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
            Some("CONNECTION_PROBE"),
        ),
    );

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtImport as i32);
    let mut close = response(MessageType::MtClose, conn_id);
    close.close = Some(exa_proto::ExascriptClose {
        exception_message: Some("import denied by db".into()),
    });
    send_resp(&server, &close);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtClose as i32);
    let close_msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        close_msg.contains("import denied by db"),
        "the DB's own MT_CLOSE message must survive into the connect-back \
         error, not be replaced by the generic not-ConnInfo text: {close_msg:?}"
    );

    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("the closed connect-back must surface as Err");
    assert!(
        err.to_string().contains("import denied by db"),
        "runtime error carries the DB's close message: {err}"
    );
}

#[cfg(feature = "connect-back")]
#[test]
fn adapter_connection_probe_combines_hook_and_recorded_errors() {
    // invoke_vs_adapter_call's Some(detail) sub-arm: when the hook fails *and*
    // it called ctx.connection(...), the connect-back error recorded on the
    // context is folded into the surfaced message alongside the hook's own.
    let so = fixture_so_path("single_call_fixture");
    let conn_id = 113u64;
    let source = format!("%udf_object {}", so.display());
    let endpoint = endpoint_for("connectionprobe");

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
            Some("CONNECTION_PROBE"),
        ),
    );

    // The hook blocks mid-call on ctx.connection("PROBE_CONN"), which sends its
    // own MT_IMPORT. Answer with anything that isn't ConnInfo (here MT_CLEANUP)
    // so the connect-back fails and records an error on the context.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtImport as i32,
        "the connection probe must issue its own MT_IMPORT mid-call"
    );
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "the combined hook+connect-back error must close the wire with MT_CLOSE"
    );
    let close_msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        close_msg.contains("VS_ADAPTER_ERROR"),
        "close carries the hook's own error text: {close_msg:?}"
    );
    assert!(
        close_msg.contains("Connect-back error"),
        "close carries the recorded connect-back detail: {close_msg:?}"
    );

    let result = client.join().expect("client thread panicked");
    let err = result.expect_err("the combined error must surface as Err");
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("VS_ADAPTER_ERROR") && err_msg.contains("Connect-back error"),
        "runtime error combines both the hook's and the recorded connect-back error: {err_msg:?}"
    );
}
