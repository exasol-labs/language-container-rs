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
