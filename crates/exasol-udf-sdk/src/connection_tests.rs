use super::*;

#[test]
fn connection_object_fields_public_unconditional() {
    // ConnectionObject must be constructible and readable without any feature gate.
    let obj = ConnectionObject {
        kind: "EXA".into(),
        address: "192.0.2.1:8563".into(),
        user: "sys".into(),
        password: "secret".into(),
    };
    assert_eq!(obj.kind, "EXA");
    assert_eq!(obj.address, "192.0.2.1:8563");
    assert_eq!(obj.user, "sys");
    assert_eq!(obj.password, "secret");
}
