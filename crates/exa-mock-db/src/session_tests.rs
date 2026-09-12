use super::*;

#[test]
fn peek_type_reads_the_leading_type_field_and_rejects_foreign_bytes() {
    for mt in [
        MessageType::MtClient,
        MessageType::MtNext,
        MessageType::MtEmit,
        MessageType::MtDone,
        MessageType::MtUndefinedCall,
    ] {
        let req = ExascriptRequest {
            r#type: mt as i32,
            connection_id: MOCK_CONN_ID,
            ..Default::default()
        };
        assert_eq!(peek_type(&req.encode_to_vec()), Some(mt as i32));
    }
    assert_eq!(peek_type(&[]), None);
    assert_eq!(peek_type(&[0x10, 0x01]), None);
    assert_eq!(peek_type(&[0x08]), None);
}

#[test]
fn bind_creates_a_removable_ipc_socket() {
    let path = {
        let s = Session::bind("unit").unwrap();
        assert!(s.endpoint().starts_with("ipc://"));
        assert!(s.ipc_path.exists());
        s.ipc_path.clone()
    };
    assert!(!path.exists());
}
