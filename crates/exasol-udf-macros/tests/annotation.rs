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
fn entry_point_symbol_is_named() {
    // fn annotated_udf → SQL name ANNOTATED_UDF → symbol __exa_udf_entry_ANNOTATED_UDF
    // This test verifies the symbol exists and returns a non-null vtable.
    let vt_ptr = __exa_udf_entry_ANNOTATED_UDF();
    assert!(
        !vt_ptr.is_null(),
        "entry point must return a non-null vtable"
    );
}

#[test]
fn annotation_maps_types_to_exatype() {
    let vt = unsafe { &*__exa_udf_entry_ANNOTATED_UDF() };
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
    let vt = unsafe { &*__exa_udf_entry_ANNOTATED_UDF() };
    let input = schema_str(vt.annotated_input_schema).unwrap();
    let output = schema_str(vt.annotated_output_schema).unwrap();

    assert_eq!(
        input,
        r#"[{"name":"a","type":"Int32"},{"name":"b","type":"Int64"},{"name":"c","type":"Double"},{"name":"d","type":"Double"},{"name":"e","type":"Boolean"},{"name":"f","type":"String"},{"name":"g","type":"String"}]"#
    );
    assert_eq!(output, r#"[{"name":"result","type":"Int64"}]"#);
}

#[test]
fn fn_name_uppercased_to_sql_name() {
    // The macro must translate snake_case fn ident to UPPER_SNAKE_CASE SQL name.
    // __exa_udf_entry_ANNOTATED_UDF must exist (no bare __exa_udf_entry symbol).
    let vt = unsafe { &*__exa_udf_entry_ANNOTATED_UDF() };
    // The vtable is valid — abi_version is set to EXA_UDF_ABI_VERSION.
    assert_ne!(vt.abi_version, 0, "abi_version must be non-zero");
}

// ---- name attribute override ----

#[exasol_udf(name = "MY_CUSTOM")]
fn custom_named(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[test]
fn name_attribute_overrides_entry_name() {
    // The `name = "MY_CUSTOM"` attribute must produce __exa_udf_entry_MY_CUSTOM,
    // not __exa_udf_entry_CUSTOM_NAMED (the fn ident uppercased).
    let vt_ptr = __exa_udf_entry_MY_CUSTOM();
    assert!(
        !vt_ptr.is_null(),
        "name-overridden entry must return a non-null vtable"
    );
}

// ---- Decimal type annotation ----

#[exasol_udf(input(x: Decimal), emits(result: Decimal))]
fn decimal_udf(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[test]
fn decimal_annotation_embeds_schema() {
    let vt = unsafe { &*__exa_udf_entry_DECIMAL_UDF() };
    let input = schema_str(vt.annotated_input_schema).unwrap();
    let output = schema_str(vt.annotated_output_schema).unwrap();

    assert_eq!(input, r#"[{"name":"x","type":"Numeric"}]"#);
    assert_eq!(output, r#"[{"name":"result","type":"Numeric"}]"#);
}

// ---- two distinct names ----

#[exasol_udf]
fn double_it(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[exasol_udf]
fn triple_it(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

#[test]
fn two_distinct_names_export_two_entries() {
    // Two annotations with distinct derived names must each produce an independent
    // non-null entry point symbol.
    let vt_double = unsafe { &*__exa_udf_entry_DOUBLE_IT() };
    let vt_triple = unsafe { &*__exa_udf_entry_TRIPLE_IT() };

    // Each vtable must point to its own run shim (function pointers differ).
    assert_ne!(
        vt_double.run as usize, vt_triple.run as usize,
        "each UDF must have its own run shim"
    );
    // Both must have valid abi_versions.
    assert_ne!(vt_double.abi_version, 0);
    assert_ne!(vt_triple.abi_version, 0);
}
