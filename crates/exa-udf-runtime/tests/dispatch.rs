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

/// Path to a debug-built fixture `.so` (the dispatch harness dlopens debug).
fn so_path(lib: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push(format!("target/debug/lib{lib}.so"));
    p
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
    assert!(so.exists(), "build fixture first: {:?}", so);

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
        &so_path("scalar_double"),
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
        &so_path("set_sum"),
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
        &so_path("scalar_double"),
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
        &so_path("set_sum"),
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
        &so_path("scalar_next_illegal"),
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
        &so_path("scalar_double"),
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
        &so_path("returns_with_emit"),
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
        &so_path("emit_k"),
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
        &so_path("emit_k"),
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
