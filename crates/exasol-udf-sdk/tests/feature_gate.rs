#![cfg(not(feature = "connect-back"))]

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

// When the `connect-back` feature is disabled, the connect-back methods must
// not exist on the trait. This test compiles only because `Ctx` does not need
// to provide `exa` / `exa_named` / `exa_connect` — proving they are absent from
// the trait surface. A compile-time guard backs this up.
#[test]
fn connect_back_methods_absent_without_feature() {
    let mut ctx = Ctx;
    assert_eq!(ctx.num_columns(), 0);
    assert!(!ctx.next().unwrap());
}
