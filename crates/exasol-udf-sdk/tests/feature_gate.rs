use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

struct Ctx;

impl UdfContext for Ctx {
    fn num_columns(&self) -> usize {
        0
    }
    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Type("none".into()))
    }
    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }
}

/// `emit_record_batch_ipc` is always declared on `UdfContext` regardless of
/// the `emit-arrow` feature. Its default returns `Unimplemented`. This test
/// compiles and passes under default features (no `emit-arrow`), proving the
/// vtable slot is stable across all feature combinations.
#[test]
fn emit_record_batch_ipc_present_without_emit_arrow() {
    let mut ctx = Ctx;
    assert!(matches!(
        ctx.emit_record_batch_ipc(&[]),
        Err(UdfError::Unimplemented(_))
    ));
}

/// `cluster_ip`, `connection`, and `connect_back` are always declared on
/// `UdfContext` — no `connect-back` feature gate exists any more.
/// Their defaults return `Unimplemented`.
#[test]
fn connect_back_methods_always_present() {
    use exasol_udf_sdk::connect_back::ConnectionObject;
    let mut ctx = Ctx;
    assert!(matches!(ctx.cluster_ip(), Err(UdfError::Unimplemented(_))));
    assert!(matches!(
        ctx.connection("X"),
        Err(UdfError::Unimplemented(_))
    ));
    let obj = ConnectionObject {
        kind: "EXA".into(),
        address: "127.0.0.1:8563".into(),
        user: "u".into(),
        password: "p".into(),
    };
    assert!(matches!(
        ctx.connect_back(&obj),
        Err(UdfError::Unimplemented(_))
    ));
}
