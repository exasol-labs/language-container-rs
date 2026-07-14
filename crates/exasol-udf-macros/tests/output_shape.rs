use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::abi::OutputShape;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

// A unit-returning fn is EMITS: it produces rows via `ctx.emit`.
#[exasol_udf]
fn emits_shape(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

// An `Option<T>`-returning fn is RETURNS: it yields one value per invocation.
#[exasol_udf]
fn returns_shape(_ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    Ok(None)
}

// RETURNS UDFs whose run bodies return `None`/`Some` respectively, used to
// verify the shim passes the `Option` through to `set_return` unchanged
// instead of collapsing `None` into `Some(Value::Null)`.
#[exasol_udf]
fn returns_none(_ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    Ok(None)
}

#[exasol_udf]
fn returns_some(_ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    Ok(Some(7))
}

// Captures whatever the shim passes to `set_return`.
#[derive(Default)]
struct RecordingCtx {
    captured: Option<Option<Value>>,
}

impl UdfContext for RecordingCtx {
    fn num_columns(&self) -> usize {
        0
    }
    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Type("no columns".into()))
    }
    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }
    fn set_return(&mut self, value: Option<Value>) -> Result<(), UdfError> {
        self.captured = Some(value);
        Ok(())
    }
}

fn run_shim(
    vt_run: unsafe extern "C" fn(*mut std::ffi::c_void, *mut *mut std::ffi::c_char) -> i32,
    ctx: &mut RecordingCtx,
) {
    let mut dyn_ref: &mut dyn UdfContext = ctx;
    let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;
    let mut error_out: *mut std::ffi::c_char = std::ptr::null_mut();
    let rc = unsafe { (vt_run)(ctx_ptr, &mut error_out) };
    assert_eq!(rc, 0, "run must succeed");
    assert!(error_out.is_null());
}

#[test]
fn returns_none_passes_none_to_set_return_not_some_null() {
    // Regression test: the RETURNS shim used to route `Option<T>` through
    // `IntoValue` as a whole, collapsing `None` to `Value::Null` and then
    // rewrapping it in `Some(..)` before calling `set_return` — so
    // `set_return` never actually observed `None`, contradicting its
    // documented `None` → SQL NULL contract. The shim now maps the inner
    // value, preserving `None` as `None`.
    let vt = unsafe { &*__exa_udf_entry_RETURNS_NONE() };
    let mut ctx = RecordingCtx::default();
    run_shim(vt.run, &mut ctx);
    assert_eq!(
        ctx.captured,
        Some(None),
        "None must reach set_return as None, not Some(Value::Null)"
    );
}

#[test]
fn returns_some_passes_converted_value_to_set_return() {
    let vt = unsafe { &*__exa_udf_entry_RETURNS_SOME() };
    let mut ctx = RecordingCtx::default();
    run_shim(vt.run, &mut ctx);
    assert_eq!(ctx.captured, Some(Some(Value::Int64(7))));
}

#[test]
fn macro_derives_emits_and_returns_shape() {
    // The macro derives the output shape from the function's return type and
    // stamps the corresponding marker into the generated vtable: unit `Ok(())`
    // ⇒ EMITS, `Ok(Option<T>)` ⇒ RETURNS.
    let emits_vt = unsafe { &*__exa_udf_entry_EMITS_SHAPE() };
    let returns_vt = unsafe { &*__exa_udf_entry_RETURNS_SHAPE() };

    assert_eq!(
        emits_vt.output_shape,
        OutputShape::Emits,
        "a unit-return fn must stamp OutputShape::Emits"
    );
    assert_eq!(
        returns_vt.output_shape,
        OutputShape::Returns,
        "an Option<T>-return fn must stamp OutputShape::Returns"
    );
}
