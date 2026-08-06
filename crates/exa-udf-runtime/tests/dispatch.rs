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

mod common;
use common::fixture_cdylib_path;

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
    let so = fixture_cdylib_path("scalar_double");

    let endpoint = format!("ipc:///tmp/exa-mockdb-{}.ipc", std::process::id());
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let conn_id = 7u64;
    let source = format!("%udf_object {}", so.display());

    let ep = endpoint.clone();
    let client = std::thread::spawn(move || Runtime::new(ep, "test-client".into()).run(|_| {}));

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

    // 5. MT_NEXT -> MT_DONE (input exhausted). The emit buffer is group-scoped
    //    and flushes at the group boundary, so the client probes for the next
    //    batch (and learns the group ended) before flushing its output — the
    //    reverse of the old per-batch model, which flushed each batch's residual
    //    before probing.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtDone, conn_id));

    // 6. MT_EMIT (tail flush) -> MT_EMIT ack. Verify the emitted value is 42.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtEmit as i32, "expected MT_EMIT");
    let emitted = req.emit.expect("emit payload").table;
    assert_eq!(emitted.rows, 1);
    assert_eq!(emitted.data_int64, vec![42], "double_it(21) should emit 42");
    send_resp(&server, &response(MessageType::MtEmit, conn_id));

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

// ---------------------------------------------------------------------------
// Reactive mock-DB harness
//
// The real database binds a REP socket and reacts to whatever request the
// client (the runtime) sends. `drive_session` mirrors that: it replies to each
// request by message type rather than scripting a fixed sequence, so one
// harness drives every dispatch shape (scalar/set × returns/emits) and the
// error-close paths. It records every MT_EMIT payload and any MT_CLOSE message,
// and joins the client to report whether the session ended in error.
// ---------------------------------------------------------------------------

/// The one connection id every mock-DB session uses.
const MOCK_CONN_ID: u64 = 7;

/// Bring up one mock-DB session for a fixture `.so`: binds a REP socket scoped
/// by `tag`, spawns the [`Runtime`] client thread, and drives the handshake so
/// the caller can script the run cycle from the first `MT_RUN`.
fn start_mock_session(
    lib: &str,
    tag: &str,
    meta: ExascriptMetadata,
) -> (
    zmq::Socket,
    std::thread::JoinHandle<Result<(), exa_udf_runtime::RuntimeError>>,
) {
    let so = fixture_cdylib_path(lib);

    let endpoint = format!("ipc:///tmp/exa-mockdb-{tag}-{}.ipc", std::process::id());
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let source = format!("%udf_object {}", so.display());
    let script_name = lib.to_uppercase();

    let ep = endpoint.clone();
    let client = std::thread::spawn(move || Runtime::new(ep, "test-client".into()).run(|_| {}));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtClient as i32);
    let mut info = response(MessageType::MtInfo, MOCK_CONN_ID);
    info.info = Some(ExascriptInfo {
        source_code: source,
        script_name,
        ..Default::default()
    });
    send_resp(&server, &info);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtMeta as i32);
    let mut m = response(MessageType::MtMeta, MOCK_CONN_ID);
    m.meta = Some(meta);
    send_resp(&server, &m);

    (server, client)
}

/// One int64 input column `x` and one int64 output column `y`, with the given
/// iteration axes.
fn int64_meta(input_iter: IterType, output_iter: IterType) -> ExascriptMetadata {
    ExascriptMetadata {
        input_iter_type: input_iter as i32,
        output_iter_type: output_iter as i32,
        input_columns: vec![int64_col("x")],
        output_columns: vec![int64_col("y")],
        single_call_mode: false,
    }
}

/// Build one MT_NEXT input batch from a column of nullable i64 cells.
fn int64_batch(vals: &[Option<i64>]) -> ExascriptTableData {
    let data_int64: Vec<i64> = vals.iter().filter_map(|v| *v).collect();
    let data_nulls: Vec<bool> = vals.iter().map(|v| v.is_none()).collect();
    ExascriptTableData {
        rows: vals.len() as u64,
        rows_in_group: 0,
        data_int64,
        data_nulls,
        ..Default::default()
    }
}

struct SessionOutcome {
    emits: Vec<ExascriptTableData>,
    close: Option<String>,
    errored: bool,
}

/// Drive one full UDF session against `script_name`'s `.so`, feeding `batches`
/// as the group's MT_NEXT responses and reacting to every request the runtime
/// makes until it finishes or closes.
fn drive_session(
    script_name: &str,
    so: &std::path::Path,
    meta: ExascriptMetadata,
    batches: Vec<ExascriptTableData>,
) -> SessionOutcome {
    let endpoint = format!(
        "ipc:///tmp/exa-mockdb-{}-{}-{}.ipc",
        script_name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let conn_id = 7u64;
    let source = format!("%udf_object {}", so.display());
    let script = script_name.to_string();

    let ep = endpoint.clone();
    let client = std::thread::spawn(move || Runtime::new(ep, "test-client".into()).run(|_| {}));

    let mut run_seen = 0usize;
    let mut cursor = 0usize;
    let mut emits = Vec::new();
    let mut close = None;

    loop {
        let req = recv_req(&server);
        let mt = req.r#type;
        if mt == MessageType::MtClient as i32 {
            let mut info = response(MessageType::MtInfo, conn_id);
            info.info = Some(ExascriptInfo {
                source_code: source.clone(),
                script_name: script.clone(),
                ..Default::default()
            });
            send_resp(&server, &info);
        } else if mt == MessageType::MtMeta as i32 {
            let mut m = response(MessageType::MtMeta, conn_id);
            m.meta = Some(meta.clone());
            send_resp(&server, &m);
        } else if mt == MessageType::MtRun as i32 {
            // First MT_RUN opens the single group; the second ends the session.
            run_seen += 1;
            let reply = if run_seen == 1 {
                MessageType::MtRun
            } else {
                MessageType::MtCleanup
            };
            send_resp(&server, &response(reply, conn_id));
        } else if mt == MessageType::MtNext as i32 {
            if cursor < batches.len() {
                let mut next = response(MessageType::MtNext, conn_id);
                next.next = Some(ExascriptNextDataRep {
                    table: batches[cursor].clone(),
                });
                cursor += 1;
                send_resp(&server, &next);
            } else {
                send_resp(&server, &response(MessageType::MtDone, conn_id));
            }
        } else if mt == MessageType::MtEmit as i32 {
            emits.push(req.emit.expect("emit payload").table);
            send_resp(&server, &response(MessageType::MtEmit, conn_id));
        } else if mt == MessageType::MtDone as i32 {
            send_resp(&server, &response(MessageType::MtDone, conn_id));
        } else if mt == MessageType::MtFinished as i32 {
            send_resp(&server, &response(MessageType::MtFinished, conn_id));
            break;
        } else if mt == MessageType::MtClose as i32 {
            close = req.close.and_then(|c| c.exception_message);
            break;
        } else {
            panic!("unexpected request type from client: {mt}");
        }
    }

    let result = client.join().expect("client thread panicked");
    SessionOutcome {
        emits,
        close,
        errored: result.is_err(),
    }
}

/// Concatenate every emitted batch's int64 column into one row-ordered vec, and
/// return the total emitted row count.
fn collect_int64_emits(emits: &[ExascriptTableData]) -> (Vec<i64>, u64) {
    let mut vals = Vec::new();
    let mut rows = 0u64;
    for t in emits {
        vals.extend_from_slice(&t.data_int64);
        rows += t.rows;
    }
    (vals, rows)
}

#[test]
fn scalar_dispatch_invokes_run_per_row() {
    // Bug 1 guard: scalar dispatch invokes run() once per input row, across
    // multiple MT_NEXT batches — not once per batch. scalar-double is SCALAR
    // RETURNS, so each row's returned value flows through set_return into the
    // group-scoped buffer and one tail MT_EMIT carries all rows.
    let outcome = drive_session(
        "SCALAR_DOUBLE",
        &fixture_cdylib_path("scalar_double"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![int64_batch(&[Some(10), Some(11)]), int64_batch(&[Some(12)])],
    );

    assert!(
        !outcome.errored,
        "session must succeed: {:?}",
        outcome.close
    );
    let (vals, rows) = collect_int64_emits(&outcome.emits);
    assert_eq!(rows, 3, "one output row per input row across both batches");
    assert_eq!(vals, vec![20, 22, 24], "run() ran once per row, in order");
}

#[test]
fn set_dispatch_next_spans_batches() {
    // Bug 2 guard: set dispatch invokes run() once per group; ctx.next() spans
    // every MT_NEXT batch and returns false only at the group boundary, so the
    // aggregate covers the whole group. set-sum is SET RETURNS.
    let outcome = drive_session(
        "SET_SUM",
        &fixture_cdylib_path("set_sum"),
        int64_meta(IterType::PbMultiple, IterType::PbExactlyOnce),
        vec![
            int64_batch(&[Some(1), Some(2), Some(3)]),
            int64_batch(&[Some(4), Some(5)]),
        ],
    );

    assert!(
        !outcome.errored,
        "session must succeed: {:?}",
        outcome.close
    );
    let (vals, rows) = collect_int64_emits(&outcome.emits);
    assert_eq!(rows, 1, "a SET RETURNS group yields exactly one output row");
    assert_eq!(vals, vec![15], "sum spans both batches (1+2+3+4+5)");
}

#[test]
fn empty_group_invokes_run_zero_times_for_scalar_and_set() {
    // Empty-input contract: when a group delivers no rows (MT_NEXT answered
    // MT_DONE immediately), the dispatcher invokes run() ZERO times for BOTH
    // scalar and set, so the container emits nothing. This matches the reference
    // Exasol containers, where run() is NOT called on an empty group — verified
    // against a live DB: a PYTHON3 SET UDF that emits a sentinel unconditionally
    // in run() produces a NULL row (never the sentinel) for empty no-GROUP-BY
    // input, proving the empty-group output row is synthesized by the DB's
    // aggregate layer, not the script. The container must therefore be a clean
    // no-op here; emitting anything (e.g. by calling run() once) would be wrong.
    let scalar = drive_session(
        "SCALAR_DOUBLE",
        &fixture_cdylib_path("scalar_double"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![],
    );
    assert!(
        !scalar.errored,
        "empty scalar group must succeed: {:?}",
        scalar.close
    );
    assert!(
        scalar.emits.is_empty(),
        "scalar run() invoked zero times on an empty group → no output"
    );

    // set-sum is SET RETURNS: had run() been invoked once, it would have
    // returned Ok(Some(0)) and produced one output row. Zero emits proves run()
    // was invoked zero times.
    let set = drive_session(
        "SET_SUM",
        &fixture_cdylib_path("set_sum"),
        int64_meta(IterType::PbMultiple, IterType::PbExactlyOnce),
        vec![],
    );
    assert!(
        !set.errored,
        "empty set group must succeed: {:?}",
        set.close
    );
    assert!(
        set.emits.is_empty(),
        "set run() invoked zero times on an empty group → no output row"
    );
}

#[test]
fn scalar_next_returns_error() {
    // Bug 3 guard (input contract): a SCALAR UDF calling ctx.next() is rejected
    // by the runtime's scalar-input gate and the session closes with a
    // prefixed F-UDF-CL-RUST error rather than running to completion.
    let outcome = drive_session(
        "SCALAR_NEXT_ILLEGAL",
        &fixture_cdylib_path("scalar_next_illegal"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbMultiple),
        vec![int64_batch(&[Some(1)])],
    );

    assert!(
        outcome.errored,
        "next() in scalar context must fail the session"
    );
    let msg = outcome.close.expect("a mismatch must close the session");
    assert!(
        msg.starts_with("F-UDF-CL-RUST-"),
        "close carries the prefixed error code, got: {msg}"
    );
    assert!(
        msg.contains("scalar"),
        "close explains the scalar-context ban, got: {msg}"
    );
    assert!(outcome.emits.is_empty(), "no output before the error");
}

#[test]
fn returns_set_return_and_emit_ban() {
    // RETURNS output channel: the value the UDF returns (Some/None) is emitted
    // via set_return — Some → value cell, None → NULL cell. scalar-double
    // returns Ok(Some(2n)) for a value and Ok(None) for a NULL input.
    let outcome = drive_session(
        "SCALAR_DOUBLE",
        &fixture_cdylib_path("scalar_double"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![int64_batch(&[Some(21), None])],
    );
    assert!(
        !outcome.errored,
        "session must succeed: {:?}",
        outcome.close
    );
    assert_eq!(outcome.emits.len(), 1, "one tail flush for the group");
    let table = &outcome.emits[0];
    assert_eq!(table.rows, 2, "one RETURNS row per input row");
    assert_eq!(
        table.data_int64,
        vec![42],
        "Some(21) → 42; the NULL takes no slot"
    );
    assert_eq!(
        table.data_nulls,
        vec![false, true],
        "Some → non-null cell, None → NULL cell"
    );

    // emit() ban: a RETURNS UDF that calls ctx.emit() is rejected at call time
    // and the session closes with a prefixed error.
    let banned = drive_session(
        "RETURNS_WITH_EMIT",
        &fixture_cdylib_path("returns_with_emit"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![int64_batch(&[Some(1)])],
    );
    assert!(
        banned.errored,
        "emit() in RETURNS output must fail the session"
    );
    let msg = banned.close.expect("the ban must close the session");
    assert!(
        msg.starts_with("F-UDF-CL-RUST-"),
        "close carries the prefixed error code, got: {msg}"
    );
    assert!(
        msg.contains("emit"),
        "close explains the emit-in-RETURNS ban, got: {msg}"
    );
}

#[test]
fn output_shape_marker_mismatch_errors() {
    // The runtime validates the compiled output-shape marker against the DB's
    // output iteration type before any run. emit-k is compiled EMITS; declaring
    // it RETURNS (ExactlyOnce output) is a clear F-UDF-CL-RUST error, not a
    // mid-stream misdispatch — and it closes before any MT_RUN.
    let outcome = drive_session(
        "EMIT_K",
        &fixture_cdylib_path("emit_k"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![int64_batch(&[Some(1)])],
    );

    assert!(
        outcome.errored,
        "output-shape mismatch must fail the session"
    );
    let msg = outcome.close.expect("a mismatch must close the session");
    assert!(
        msg.starts_with("F-UDF-CL-RUST-"),
        "close carries the prefixed error code, got: {msg}"
    );
    assert!(
        msg.contains("EMITS") && msg.contains("RETURNS"),
        "close names both the compiled and registered shapes, got: {msg}"
    );
    assert!(
        outcome.emits.is_empty(),
        "no output when the shape is rejected"
    );
}

#[test]
fn emit_buffer_spans_group_and_tail_flushes() {
    // The emit buffer is scoped to the whole input group: emit-k is SCALAR
    // EMITS, so each input row emits a variable number of rows, and all of them
    // batch into a single tail MT_EMIT before the group's MT_DONE (the small
    // rows never cross the 4,000,000-byte threshold mid-group).
    let outcome = drive_session(
        "EMIT_K",
        &fixture_cdylib_path("emit_k"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbMultiple),
        vec![int64_batch(&[Some(2), Some(3)])],
    );

    assert!(
        !outcome.errored,
        "session must succeed: {:?}",
        outcome.close
    );
    assert_eq!(
        outcome.emits.len(),
        1,
        "the group-scoped buffer tail-flushes once, not per input row"
    );
    let (vals, rows) = collect_int64_emits(&outcome.emits);
    assert_eq!(rows, 5, "row0 emits 2 rows, row1 emits 3 (2+3)");
    assert_eq!(
        vals,
        vec![0, 1, 0, 1, 2],
        "each input row's emitted indices, buffered across the group"
    );
}

#[test]
fn annotated_schema_mismatch_closes_session() {
    let so = fixture_cdylib_path("annotated_fixture");

    let endpoint = format!("ipc:///tmp/exa-mockdb-schema-{}.ipc", std::process::id());
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&endpoint).unwrap();

    let conn_id = 9u64;
    let source = format!("%udf_object {}", so.display());

    let ep = endpoint.clone();
    let client = std::thread::spawn(move || Runtime::new(ep, "test-client".into()).run(|_| {}));

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
    // The fixture is EMITS (`Result<(), UdfError>` + `emits(y)`), so the meta
    // declares Multiple output; the test exercises the input column-name
    // mismatch, which the schema check catches before any run.
    meta.meta = Some(ExascriptMetadata {
        input_iter_type: IterType::PbExactlyOnce as i32,
        output_iter_type: IterType::PbMultiple as i32,
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

#[test]
fn run_error_out_pointer_text_reaches_close() {
    // invoke_run's rc!=0 WITH out-pointer text: scalar_next_illegal calls
    // ctx.next(), banned in scalar context. Pins the exact "error code {rc}:
    // {text}" formatting reaching MT_CLOSE.
    let outcome = drive_session(
        "SCALAR_NEXT_ILLEGAL",
        &fixture_cdylib_path("scalar_next_illegal"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbMultiple),
        vec![int64_batch(&[Some(1)])],
    );

    assert!(
        outcome.errored,
        "next() in scalar context must fail the session"
    );
    let msg = outcome.close.expect("a run error must close the session");
    assert!(
        msg.starts_with("F-UDF-CL-RUST-9001"),
        "close carries the UDF error close code, got: {msg}"
    );
    assert!(
        msg.contains("UDF run returned error code 1: next() is not allowed in scalar context"),
        "close carries invoke_run's exact 'code N: text' formatting sourced from the out-pointer, got: {msg}"
    );
}

#[test]
fn udf_error_closes_session_with_prefixed_message() {
    // invoke_run's rc!=0 WITHOUT out-pointer text: scalar_double's `i64::MAX`
    // arm panics inside the shim's catch_unwind, which returns rc=2 without
    // touching the error out-pointer, so only "error code {rc}" is formatted.
    let outcome = drive_session(
        "SCALAR_DOUBLE",
        &fixture_cdylib_path("scalar_double"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![int64_batch(&[Some(i64::MAX)])],
    );

    assert!(
        outcome.errored,
        "an overflow panic in run() must fail the session"
    );
    let msg = outcome.close.expect("a run error must close the session");
    assert_eq!(
        msg, "F-UDF-CL-RUST-9001: UDF error: UDF run returned error code 2",
        "close carries the prefixed message with no out-pointer text appended, got: {msg}"
    );
}

#[test]
fn mid_group_cleanup_ends_session_cleanly() {
    // batch_fetcher's Cleanup arm (GroupExit::Session): a mid-group MT_CLEANUP
    // ends the session successfully. run_group skips the tail flush on this
    // exit, so the row already processed this group has its output discarded.
    let (server, client) = start_mock_session(
        "scalar_double",
        "midcleanup",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtRun, MOCK_CONN_ID));

    // First MT_NEXT delivers one row; run() executes and buffers its output.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    let mut next = response(MessageType::MtNext, MOCK_CONN_ID);
    next.next = Some(ExascriptNextDataRep {
        table: int64_batch(&[Some(10)]),
    });
    send_resp(&server, &next);

    // Second MT_NEXT: instead of MT_DONE (group boundary) or more data, the DB
    // answers MT_CLEANUP mid-group.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtCleanup, MOCK_CONN_ID));

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_ok(),
        "mid-group MT_CLEANUP must end the session cleanly: {:?}",
        result.err()
    );
}

#[test]
fn mid_group_close_surfaces_message() {
    // batch_fetcher's Close arm (GroupExit::Closed): a mid-group MT_CLOSE
    // surfaces its exception message as the run error, relayed back to the DB
    // in the runtime's own MT_CLOSE.
    let (server, client) = start_mock_session(
        "scalar_double",
        "midclose",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtRun, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    let mut next = response(MessageType::MtNext, MOCK_CONN_ID);
    next.next = Some(ExascriptNextDataRep {
        table: int64_batch(&[Some(10)]),
    });
    send_resp(&server, &next);

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    let mut close = response(MessageType::MtClose, MOCK_CONN_ID);
    close.close = Some(exa_proto::ExascriptClose {
        exception_message: Some("mid-group boom".into()),
    });
    send_resp(&server, &close);

    // The runtime surfaces the DB's close as its own error and relays a
    // prefixed MT_CLOSE back before returning; it does not wait for a reply.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "the runtime relays the DB's mid-group close as its own MT_CLOSE"
    );
    let relayed = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        relayed.starts_with("F-UDF-CL-RUST-9001"),
        "relayed close carries the UDF error close code, got: {relayed}"
    );
    assert!(
        relayed.contains("mid-group boom"),
        "relayed close carries the DB's original exception message, got: {relayed}"
    );

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_err(),
        "mid-group MT_CLOSE must surface as a run error"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("mid-group boom"),
        "runtime error carries the DB's exception message: {err_msg:?}"
    );
}

#[test]
fn ping_pong_mid_exchange_retries_transparently() {
    // wire::request's ping-transparent retry: the client echoes a mid-exchange
    // ping and treats the DB's answer to that echo as the original request's
    // outcome. Must hold for every ping in a row, so the DB pings twice here.
    let (server, client) = start_mock_session(
        "scalar_double",
        "pingpong",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    // First MT_RUN: instead of the real answer, ping mid-exchange.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    let mut ping = response(MessageType::MtPingPong, MOCK_CONN_ID);
    ping.ping = Some(exa_proto::ExascriptPing {
        meta_info: "ping-token-1".into(),
    });
    send_resp(&server, &ping);

    // The client must echo the ping (not surface it as the MT_RUN answer)
    // rather than treat it as the MT_RUN reply.
    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtPingPong as i32,
        "client must reply to the ping before the original request is answered"
    );
    assert_eq!(
        req.ping.expect("ping").meta_info,
        "ping-token-1",
        "ping reply must echo the DB's meta_info"
    );

    // A second ping back-to-back, carrying a different token: answering the
    // client's first echo with another ping must be just as transparent.
    let mut ping = response(MessageType::MtPingPong, MOCK_CONN_ID);
    ping.ping = Some(exa_proto::ExascriptPing {
        meta_info: "ping-token-2".into(),
    });
    send_resp(&server, &ping);

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtPingPong as i32,
        "a ping answering the previous ping echo must be echoed too, not \
         surfaced as the original request's answer"
    );
    assert_eq!(
        req.ping.expect("ping").meta_info,
        "ping-token-2",
        "the second ping reply must echo the second token, not the first"
    );

    // The DB's answer to the last ping echo fulfils the original MT_RUN
    // request: end the session immediately.
    send_resp(&server, &response(MessageType::MtCleanup, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, MOCK_CONN_ID));

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_ok(),
        "ping mid-exchange must be transparent: {:?}",
        result.err()
    );
}

#[test]
fn close_after_initial_run_request_ends_session() {
    // run_udf's post-MT_RUN match Close arm: MT_CLOSE answering the very first
    // MT_RUN must surface as a run error rather than proceeding into run_group.
    let (server, client) = start_mock_session(
        "scalar_double",
        "closeonrun",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    // The very first MT_RUN is answered directly with MT_CLOSE: no group ever
    // starts.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    let mut close = response(MessageType::MtClose, MOCK_CONN_ID);
    close.close = Some(exa_proto::ExascriptClose {
        exception_message: Some("no session for you".into()),
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
        relayed.starts_with("F-UDF-CL-RUST-9001"),
        "relayed close carries the UDF error close code, got: {relayed}"
    );
    assert!(
        relayed.contains("no session for you"),
        "relayed close carries the DB's original exception message, got: {relayed}"
    );

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_err(),
        "a close before the first group must surface as a run error"
    );
}

#[test]
fn wildcard_after_run_request_is_ignored_and_cleanup_after_done_ends_session() {
    // run_udf's post-MT_RUN match: an event other than Run/Cleanup/Close (here
    // MT_TRY_AGAIN) falls into the silent `_ => {}` arm and the loop proceeds
    // into run_group as if MT_RUN had answered normally — dispatch tolerates a
    // stray event here where single-call mode hard-errors, on purpose. The DB
    // then answers the group's MT_DONE with MT_CLEANUP directly, exercising the
    // post-MT_DONE Cleanup arm.
    let (server, client) = start_mock_session(
        "scalar_double",
        "runwildcard",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    // Answer the first MT_RUN with MT_TRY_AGAIN instead of MT_RUN/MT_CLEANUP.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtTryAgain, MOCK_CONN_ID));

    // The loop proceeded into run_group as though MT_RUN had answered
    // normally: the group's own MT_NEXT arrives next. End it immediately with
    // MT_DONE (empty group, so the tail flush is a no-op).
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtDone, MOCK_CONN_ID));

    // The client's own MT_DONE is answered with MT_CLEANUP directly.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    send_resp(&server, &response(MessageType::MtCleanup, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, MOCK_CONN_ID));

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_ok(),
        "a stray event after MT_RUN must be ignored, not fail the session: {:?}",
        result.err()
    );
}

#[test]
fn close_after_done_request_ends_session() {
    // run_udf's post-MT_DONE match Close arm: MT_CLOSE can answer the client's
    // own MT_DONE, not only arrive mid-group.
    let (server, client) = start_mock_session(
        "scalar_double",
        "closeondone",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtRun, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtDone, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    let mut close = response(MessageType::MtClose, MOCK_CONN_ID);
    close.close = Some(exa_proto::ExascriptClose {
        exception_message: Some("teardown boom".into()),
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
        relayed.starts_with("F-UDF-CL-RUST-9001"),
        "relayed close carries the UDF error close code, got: {relayed}"
    );
    assert!(
        relayed.contains("teardown boom"),
        "relayed close carries the DB's original exception message, got: {relayed}"
    );

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_err(),
        "a close after the group's own MT_DONE must surface as a run error"
    );
}

#[test]
fn wildcard_after_done_request_continues_loop() {
    // run_udf's post-MT_DONE match wildcard arm: an event other than
    // Done/Cleanup/Close is ignored and the outer loop continues to a fresh
    // MT_RUN round rather than failing the session.
    let (server, client) = start_mock_session(
        "scalar_double",
        "donewildcard",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtRun, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtDone, MOCK_CONN_ID));

    // Answer the client's own MT_DONE with MT_TRY_AGAIN: ignored, loop
    // continues to a second MT_RUN round.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    send_resp(&server, &response(MessageType::MtTryAgain, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtRun as i32,
        "the loop must continue to a fresh MT_RUN round"
    );
    send_resp(&server, &response(MessageType::MtCleanup, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, MOCK_CONN_ID));

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_ok(),
        "a stray event after MT_DONE must be ignored, not fail the session: {:?}",
        result.err()
    );
}

#[test]
fn emit_buffer_flushes_mid_group_before_tail_flush() {
    // emit_flusher's mid-group MT_EMIT: pushing enough small emitted rows to
    // cross the 4,000,000-byte threshold mid-group forces a flush through
    // emit_flusher's closure body (the zero-row no-op check, the cell borrow,
    // building and sending the MT_EMIT request, and mapping the result) before
    // the group's own tail flush ever runs. Each scalar-double output row
    // costs exactly 8 bytes (one int64 cell), so 500_000 rows cross
    // EMIT_BUFFER_LIMIT_BYTES (4_000_000) exactly on push #500_000; two extra
    // rows continue past it to prove the buffer keeps accumulating for a
    // genuine tail flush afterward, rather than the mid-group flush being
    // mistaken for the group's only flush.
    const MID_GROUP_ROWS: usize = 500_000;
    const TAIL_ROWS: usize = 2;
    let vals: Vec<Option<i64>> = (0..(MID_GROUP_ROWS + TAIL_ROWS) as i64).map(Some).collect();

    let outcome = drive_session(
        "SCALAR_DOUBLE",
        &fixture_cdylib_path("scalar_double"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![int64_batch(&vals)],
    );

    assert!(
        !outcome.errored,
        "session must succeed: {:?}",
        outcome.close
    );
    assert_eq!(
        outcome.emits.len(),
        2,
        "one mid-group MT_EMIT on crossing the threshold, one tail MT_EMIT for the rest"
    );
    assert_eq!(
        outcome.emits[0].rows as usize, MID_GROUP_ROWS,
        "the mid-group flush fires exactly at the byte threshold, not before or after"
    );
    assert_eq!(
        outcome.emits[1].rows as usize, TAIL_ROWS,
        "residual rows after the mid-group flush still reach the tail flush"
    );
}

#[test]
fn first_nonempty_input_skips_leading_empty_batch() {
    // first_nonempty_input's success return after skipping an empty batch: the
    // DB may answer MT_NEXT with a zero-row table before the group's real data
    // arrives (distinct from MT_DONE, which ends the group outright). The
    // dispatcher must skip it silently and use the first row-bearing batch,
    // rather than treating the empty batch as the group boundary or invoking
    // run() on it.
    let outcome = drive_session(
        "SCALAR_DOUBLE",
        &fixture_cdylib_path("scalar_double"),
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
        vec![int64_batch(&[]), int64_batch(&[Some(7)])],
    );

    assert!(
        !outcome.errored,
        "session must succeed: {:?}",
        outcome.close
    );
    let (vals, rows) = collect_int64_emits(&outcome.emits);
    assert_eq!(rows, 1, "one output row from the first non-empty batch");
    assert_eq!(
        vals,
        vec![14],
        "run() executed on the skipped-to row, not on the empty one"
    );
}

#[test]
fn batch_fetcher_retries_on_try_again_and_ignores_unrecognized_events() {
    // batch_fetcher's own retry loop (distinct from wire::request's
    // ping-transparent retry, pinned by ping_pong_mid_exchange_retries_transparently):
    // an MT_TRY_AGAIN reply to MT_NEXT retries the same request, and a
    // reply the loop doesn't specifically match (here MT_RUN, classified as
    // HostEvent::Run) falls into the wildcard `_ => continue` arm and also
    // retries, rather than being mistaken for a batch or the group boundary.
    let (server, client) = start_mock_session(
        "scalar_double",
        "fetcherretry",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtRun, MOCK_CONN_ID));

    // 1st MT_NEXT: MT_TRY_AGAIN -> retry.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtTryAgain, MOCK_CONN_ID));

    // 2nd MT_NEXT (the retry): MT_RUN -> unmatched, wildcard retry.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtRun, MOCK_CONN_ID));

    // 3rd MT_NEXT (the second retry): the real batch.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    let mut next = response(MessageType::MtNext, MOCK_CONN_ID);
    next.next = Some(ExascriptNextDataRep {
        table: int64_batch(&[Some(5)]),
    });
    send_resp(&server, &next);

    // 4th MT_NEXT: group boundary.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtDone, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtEmit as i32, "expected MT_EMIT");
    let emitted = req.emit.expect("emit payload").table;
    assert_eq!(emitted.data_int64, vec![10], "double_it(5) should emit 10");
    send_resp(&server, &response(MessageType::MtEmit, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtDone as i32);
    send_resp(&server, &response(MessageType::MtCleanup, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtFinished as i32);
    send_resp(&server, &response(MessageType::MtFinished, MOCK_CONN_ID));

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_ok(),
        "retries must be transparent to the group: {:?}",
        result.err()
    );
}

#[test]
fn advance_row_wire_error_ends_group_as_run_error() {
    // drive_group_rows's Err arm: advance_row's refill fetches the next batch
    // through the same wire request the group's batch fetcher uses. If that
    // exchange fails at the protocol level (not a UDF error) -- here the DB
    // answers the refill's MT_NEXT with MT_CLIENT, a message type with no
    // valid arm mid-run -- the error must surface as a run error that closes
    // the session, not panic or hang.
    let (server, client) = start_mock_session(
        "scalar_double",
        "advanceerr",
        int64_meta(IterType::PbExactlyOnce, IterType::PbExactlyOnce),
    );

    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtRun as i32);
    send_resp(&server, &response(MessageType::MtRun, MOCK_CONN_ID));

    // First MT_NEXT delivers one row; run() executes and buffers its output,
    // then advance_row drains the single-row batch and refills via a second
    // MT_NEXT.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    let mut next = response(MessageType::MtNext, MOCK_CONN_ID);
    next.next = Some(ExascriptNextDataRep {
        table: int64_batch(&[Some(10)]),
    });
    send_resp(&server, &next);

    // The refill's MT_NEXT gets a reply with no arm at all mid-run (MT_CLIENT
    // is only valid pre-handshake); the protocol classifies it as a hard
    // error, which advance_row's `?` turns into an Err.
    let req = recv_req(&server);
    assert_eq!(req.r#type, MessageType::MtNext as i32);
    send_resp(&server, &response(MessageType::MtClient, MOCK_CONN_ID));

    let req = recv_req(&server);
    assert_eq!(
        req.r#type,
        MessageType::MtClose as i32,
        "a wire-protocol error mid-group must close the session"
    );
    let msg = req
        .close
        .expect("close")
        .exception_message
        .expect("exception_message");
    assert!(
        msg.starts_with("F-UDF-CL-RUST-9001"),
        "close carries the UDF error close code, got: {msg}"
    );

    let result = client.join().expect("client thread panicked");
    assert!(
        result.is_err(),
        "a mid-group wire protocol error must surface as a run error"
    );
}
