use exa_proto::{
    ExascriptClient, ExascriptNextDataRep, ExascriptRequest, ExascriptResponse, ExascriptTableData,
    MessageType,
};
use exa_zmq_protocol::ZmqTransport;
use prost::Message;
use std::time::Duration;

/// Unique IPC endpoint per test so parallel runs do not collide.
fn endpoint(tag: &str) -> String {
    let pid = std::process::id();
    format!("ipc:///tmp/exa-zmq-{tag}-{pid}.ipc")
}

fn client_request() -> ExascriptRequest {
    ExascriptRequest {
        r#type: MessageType::MtClient as i32,
        connection_id: 42,
        client: Some(ExascriptClient {
            client_name: "tcp://127.0.0.1:1".into(),
            meta_info: None,
        }),
        ..Default::default()
    }
}

#[test]
fn transport_connects_to_ipc() {
    let ep = endpoint("connect");
    let ctx = zmq::Context::new();
    // The DB binds a REP socket; the client connects a REQ.
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&ep).unwrap();

    let transport = ZmqTransport::connect(&ep);
    assert!(transport.is_ok(), "REQ should connect to bound REP");
}

#[test]
fn transport_round_trip_single_frame() {
    let ep = endpoint("roundtrip");
    let ctx = zmq::Context::new();
    // The DB side is a REP socket. REQ clients send one raw payload frame;
    // REP strips the delimiter automatically and delivers just the payload.
    // REP replies with a single frame that REQ receives as the payload.
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&ep).unwrap();

    let transport = ZmqTransport::connect(&ep).unwrap();
    transport.send(&client_request()).unwrap();

    // REP delivers just the payload frame.
    let payload = server.recv_bytes(0).unwrap();
    let decoded = ExascriptRequest::decode(payload.as_slice()).unwrap();
    assert_eq!(decoded.r#type, MessageType::MtClient as i32);
    assert_eq!(decoded.connection_id, 42);
    assert_eq!(decoded.client.unwrap().client_name, "tcp://127.0.0.1:1");

    // Reply: single frame; REP handles routing back to the REQ peer.
    let reply = ExascriptResponse {
        r#type: MessageType::MtInfo as i32,
        connection_id: 42,
        ..Default::default()
    };
    server.send(reply.encode_to_vec(), 0).unwrap();

    let got = transport.recv().unwrap();
    assert_eq!(got.r#type, MessageType::MtInfo as i32);
    assert_eq!(got.connection_id, 42);
}

/// Regression guard for the production crash signature `handleDead() ...
/// state=15; signaled=FALSE` followed by the engine SIGKILLing sibling VMs.
///
/// Under a loaded cluster the engine occasionally takes longer than the 1 s
/// `RCVTIMEO` poll interval to reply to a given VM's request (e.g. while
/// draining a large MT_EMIT stream). `recv()` must treat that ZMQ `EAGAIN`
/// timeout as transient and keep waiting, not propagate it as fatal — a fatal
/// error breaks REQ/REP lockstep and makes the VM self-terminate abnormally.
#[test]
fn recv_waits_through_a_reply_slower_than_the_poll_interval() {
    let ep = endpoint("slow-reply");
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&ep).unwrap();

    let transport = ZmqTransport::connect(&ep).unwrap();
    transport.send(&client_request()).unwrap();

    // The peer receives the request promptly but stalls ~2 s before replying —
    // well past the 1 s RCVTIMEO so the client's first poll cycle returns EAGAIN.
    let payload = server.recv_bytes(0).unwrap();
    assert_eq!(
        ExascriptRequest::decode(payload.as_slice()).unwrap().r#type,
        MessageType::MtClient as i32
    );
    let server_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2000));
        let reply = ExascriptResponse {
            r#type: MessageType::MtInfo as i32,
            connection_id: 42,
            ..Default::default()
        };
        server.send(reply.encode_to_vec(), 0).unwrap();
    });

    // Must succeed despite the >1 s stall, not error on the EAGAIN timeout.
    let got = transport.recv().unwrap();
    assert_eq!(got.r#type, MessageType::MtInfo as i32);
    assert_eq!(got.connection_id, 42);
    server_thread.join().unwrap();
}

/// `send` must re-encode the request as a fresh `zmq::Message` on every retry
/// attempt, since `Socket::send` consumes the message it is given. A REQ
/// socket with no connected peer yet returns `EAGAIN` from the `SNDTIMEO`
/// poll interval, so connecting before the peer binds exercises that retry.
#[test]
fn send_retries_through_a_peer_that_binds_after_the_poll_interval() {
    let ep = endpoint("late-bind-send");
    let transport = ZmqTransport::connect(&ep).unwrap();

    let ep_for_server = ep.clone();
    let server_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let ctx = zmq::Context::new();
        let server = ctx.socket(zmq::REP).unwrap();
        server.bind(&ep_for_server).unwrap();
        let payload = server.recv_bytes(0).unwrap();
        ExascriptRequest::decode(payload.as_slice()).unwrap()
    });

    transport.send(&client_request()).unwrap();
    let decoded = server_thread.join().unwrap();
    assert_eq!(decoded.r#type, MessageType::MtClient as i32);
    assert_eq!(decoded.connection_id, 42);
}

/// `recv` decodes directly from `recv_msg`'s byte slice rather than a copied
/// `Vec<u8>`; exercise it against a multi-field, non-trivial payload.
#[test]
fn recv_decodes_a_multi_field_response_from_the_message_slice() {
    let ep = endpoint("slice-decode");
    let ctx = zmq::Context::new();
    let server = ctx.socket(zmq::REP).unwrap();
    server.bind(&ep).unwrap();

    let transport = ZmqTransport::connect(&ep).unwrap();
    transport.send(&client_request()).unwrap();
    server.recv_bytes(0).unwrap();

    let reply = ExascriptResponse {
        r#type: MessageType::MtNext as i32,
        connection_id: 42,
        next: Some(ExascriptNextDataRep {
            table: ExascriptTableData {
                rows: 3,
                rows_in_group: 3,
                data_string: vec!["a".into(), "bb".into(), "ccc".repeat(1000)],
                ..Default::default()
            },
        }),
        ..Default::default()
    };
    server.send(reply.encode_to_vec(), 0).unwrap();

    let got = transport.recv().unwrap();
    assert_eq!(got.r#type, MessageType::MtNext as i32);
    let table = got.next.unwrap().table;
    assert_eq!(table.rows, 3);
    assert_eq!(table.data_string, vec!["a", "bb", &"ccc".repeat(1000)]);
}
