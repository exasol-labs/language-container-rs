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
    // The DB binds a ROUTER socket; the client connects a DEALER.
    let server = ctx.socket(zmq::ROUTER).unwrap();
    server.bind(&ep).unwrap();

    let transport = ZmqTransport::connect(&ep);
    assert!(transport.is_ok(), "DEALER should connect to bound ROUTER");
}

#[test]
fn transport_round_trip_single_frame() {
    let ep = endpoint("roundtrip");
    let ctx = zmq::Context::new();
    // The DB side is a ROUTER socket. DEALER clients send one raw payload
    // frame; ROUTER prepends the sender identity when delivering to the
    // application ([identity, payload]). To reply, the server re-attaches the
    // identity so ROUTER can route back. The DEALER sees just the payload
    // frame — one prost message in, one prost message out.
    let server = ctx.socket(zmq::ROUTER).unwrap();
    server.bind(&ep).unwrap();

    let transport = ZmqTransport::connect(&ep).unwrap();
    transport.send(&client_request()).unwrap();

    // ROUTER delivers [identity, empty, payload]; capture identity for reply.
    let identity = server.recv_bytes(0).unwrap();
    assert!(
        server.get_rcvmore().unwrap(),
        "expected more frames after identity"
    );
    let empty = server.recv_bytes(0).unwrap();
    assert!(empty.is_empty(), "expected empty delimiter frame");
    assert!(
        server.get_rcvmore().unwrap(),
        "expected payload after delimiter"
    );
    let payload = server.recv_bytes(0).unwrap();
    let decoded = ExascriptRequest::decode(payload.as_slice()).unwrap();
    assert_eq!(decoded.r#type, MessageType::MtClient as i32);
    assert_eq!(decoded.connection_id, 42);
    assert_eq!(decoded.client.unwrap().client_name, "tcp://127.0.0.1:1");

    // Reply: [identity, empty, payload] so ROUTER routes back to our DEALER.
    let reply = ExascriptResponse {
        r#type: MessageType::MtInfo as i32,
        connection_id: 42,
        ..Default::default()
    };
    server.send(identity, zmq::SNDMORE).unwrap();
    server.send(b"" as &[u8], zmq::SNDMORE).unwrap();
    server.send(reply.encode_to_vec(), 0).unwrap();

    // DEALER receives [empty, payload]; recv() discards the empty delimiter.
    let got = transport.recv().unwrap();
    assert_eq!(got.r#type, MessageType::MtInfo as i32);
    assert_eq!(got.connection_id, 42);
}
