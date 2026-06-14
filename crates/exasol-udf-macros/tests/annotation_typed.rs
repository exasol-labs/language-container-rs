use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use std::ffi::CStr;

#[exasol_udf(
    input(d: Decimal, date: NaiveDate, ts: NaiveDateTime),
    emits(out: Decimal)
)]
fn typed_udf(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn schema_str(ptr: *const std::ffi::c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: the macro emits a NUL-terminated 'static string literal.
        Some(
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

#[test]
fn macro_maps_decimal_date_timestamp() {
    let vt = unsafe { &*__exa_udf_entry() };
    let input = schema_str(vt.annotated_input_schema).unwrap();
    let output = schema_str(vt.annotated_output_schema).unwrap();
    assert_eq!(
        input,
        r#"[{"name":"d","type":"Numeric"},{"name":"date","type":"Date"},{"name":"ts","type":"Timestamp"}]"#
    );
    assert_eq!(output, r#"[{"name":"out","type":"Numeric"}]"#);
}
