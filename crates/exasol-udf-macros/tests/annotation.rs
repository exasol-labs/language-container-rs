use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use std::ffi::CStr;

#[exasol_udf(
    input(a: i32, b: i64, c: f64, d: f32, e: bool, f: String, g: &str),
    emits(result: i64)
)]
fn annotated_udf(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
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
fn annotation_maps_types_to_exatype() {
    let vt = unsafe { &*__exa_udf_entry() };
    assert!(
        !vt.annotated_input_schema.is_null(),
        "annotated input schema must be non-null"
    );
    assert!(
        !vt.annotated_output_schema.is_null(),
        "annotated output schema must be non-null"
    );
}

#[test]
fn annotated_double_embeds_schema() {
    let vt = unsafe { &*__exa_udf_entry() };
    let input = schema_str(vt.annotated_input_schema).unwrap();
    let output = schema_str(vt.annotated_output_schema).unwrap();

    assert_eq!(
        input,
        r#"[{"name":"a","type":"Int32"},{"name":"b","type":"Int64"},{"name":"c","type":"Double"},{"name":"d","type":"Double"},{"name":"e","type":"Boolean"},{"name":"f","type":"String"},{"name":"g","type":"String"}]"#
    );
    assert_eq!(output, r#"[{"name":"result","type":"Int64"}]"#);
}
