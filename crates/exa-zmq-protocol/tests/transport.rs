use exa_proto::{ExascriptClient, ExascriptRequest, ExascriptResponse, MessageType};
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
