use exa_proto::{ExascriptClient, ExascriptRequest, ExascriptResponse, MessageType};
use exa_zmq_protocol::ZmqTransport;
use prost::Message;

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
