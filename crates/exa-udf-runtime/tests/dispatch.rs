//! End-to-end dispatch test against a mock database.
//!
//! Binds a ZMQ `REP` socket (the role the real database plays for a local
//! `ipc://` client) and replays the wire handshake and run-cycle protocol while
//! the real [`Runtime`] drives a loaded `libscalar_double.so`. This pins the
//! exact request/reply ordering the database expects, without Docker.

use exa_proto::exascript_metadata::ColumnDefinition;
use exa_proto::{ColumnType, IterType};
use exa_proto::{
    ExascriptInfo, ExascriptMetadata, ExascriptNextDataRep, ExascriptResponse, ExascriptTableData,
    MessageType,
};
use exa_udf_runtime::Runtime;
use prost::Message;
use std::path::PathBuf;

fn scalar_so_path() -> PathBuf {
    // tests run with CWD at the crate root; the workspace target dir is two up.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target/debug/libscalar_double.so");
    p
}

fn int64_col(name: &str) -> ColumnDefinition {
    ColumnDefinition {
        name: name.into(),
        r#type: Some(ColumnType::PbInt64 as i32),
        type_name: "BIGINT".into(),
        size: None,
        precision: None,
        scale: None,
    }
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

#[test]
fn scalar_dispatch_full_protocol() {
    let so = scalar_so_path();
    assert!(so.exists(), "build libscalar_double.so first: {:?}", so);

    let endpoint = format!("ipc:///tmp/exa-mockdb-{}.ipc", std::process::id());
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let conn_id = 7u64;
    let source = format!("%udf_object {}", so.display());

    let ep = endpoint.clone();
    let client = std::thread::spawn(move || Runtime::new(ep, "test-client".into()).run());

    // 1. MT_CLIENT -> MT_INFO
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtClient as i32);
    let mut info = response(MessageType::MtInfo, conn_id);
    info.info = Some(ExascriptInfo {
        source_code: source,
        script_name: "SCALAR_DOUBLE".into(),
        ..Default::default()
    });
    send_resp(&server, &info);

    // 2. MT_META (request) -> MT_META (response with column defs)
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtMeta as i32);
    let mut meta = response(MessageType::MtMeta, conn_id);
    meta.meta = Some(ExascriptMetadata {
        input_iter_type: IterType::PbExactlyOnce as i32,
        output_iter_type: IterType::PbExactlyOnce as i32,
        input_columns: vec![int64_col("x")],
        output_columns: vec![int64_col("y")],
        single_call_mode: false,
    });
    send_resp(&server, &meta);

    // 3. MT_RUN -> MT_RUN
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtRun, conn_id));

    // 4. MT_NEXT -> MT_NEXT with one row: x = 21
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    let mut next = response(MessageType::MtNext, conn_id);
    next.next = Some(ExascriptNextDataRep {
        table: ExascriptTableData {
            rows: 1,
            rows_in_group: 0,
            data_int64: vec![21],
            data_nulls: vec![false],
            ..Default::default()
        },
    });
    send_resp(&server, &next);

    // 5. MT_EMIT (output) -> MT_EMIT ack. Verify the emitted value is 42.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtEmit as i32, "expected MT_EMIT");
    let emitted = req.emit.expect("emit payload").table;
    assert_eq!(emitted.rows, 1);
    assert_eq!(emitted.data_int64, vec![42], "double_it(21) should emit 42");
    send_resp(&server, &response(MessageType::MtEmit, conn_id));

    // 6. MT_NEXT -> MT_DONE (input exhausted)
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtDone, conn_id));

    // 7. client sends MT_DONE -> MT_DONE
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    send_resp(&server, &response(MessageType::MtDone, conn_id));

    // 8. client opens another run cycle with MT_RUN -> MT_CLEANUP ends it
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtCleanup, conn_id));

    // 9. client sends MT_FINISHED -> MT_FINISHED
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, conn_id));

    let result = client.join().expect("client thread panicked");
    assert!(result.is_ok(), "runtime returned error: {:?}", result.err());
}

fn annotated_so_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target/debug/libannotated_fixture.so");
    p
}

#[test]
fn annotated_schema_mismatch_closes_session() {
    let so = annotated_so_path();
    assert!(so.exists(), "build libannotated_fixture.so first: {:?}", so);

    let endpoint = format!("ipc:///tmp/exa-mockdb-schema-{}.ipc", std::process::id());
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let conn_id = 9u64;
    let source = format!("%udf_object {}", so.display());

    let ep = endpoint.clone();
    let client = std::thread::spawn(move || Runtime::new(ep, "test-client".into()).run());

    // Handshake.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtClient as i32);
    let mut info = response(MessageType::MtInfo, conn_id);
    info.info = Some(ExascriptInfo {
        source_code: source,
        script_name: "ANNOTATED".into(),
        ..Default::default()
    });
    send_resp(&server, &info);

    // The fixture annotates input column `x`, but the DB advertises `wrong`.
    // The runtime must reject the session at load time, before any MT_RUN.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtMeta as i32);
    let mut meta = response(MessageType::MtMeta, conn_id);
    meta.meta = Some(ExascriptMetadata {
        input_iter_type: IterType::PbExactlyOnce as i32,
        output_iter_type: IterType::PbExactlyOnce as i32,
        input_columns: vec![int64_col("wrong")],
        output_columns: vec![int64_col("y")],
        single_call_mode: false,
    });
    send_resp(&server, &meta);

    // The next message must be MT_CLOSE carrying the schema-mismatch code.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "schema mismatch must close the session, not start the run loop"
    );
    let msg = req
        .close
        .and_then(|c| c.exception_message)
        .expect("close carries an exception message");
    assert!(
        msg.starts_with("F-UDF-CL-RUST-1001"),
        "close message must carry the schema-mismatch code, got: {msg}"
    );

    let result = client.join().expect("client thread panicked");
    assert!(result.is_err(), "runtime must surface the schema mismatch");
}
