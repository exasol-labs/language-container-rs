use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Ident, ItemFn, Path, Token, Type};

/// A single `name: Type` field inside an `input(...)` or `emits(...)` list.
struct SchemaField {
    name: Ident,
    ty: Type,
}

impl Parse for SchemaField {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let ty: Type = input.parse()?;
        Ok(SchemaField { name, ty })
    }
}

/// The parsed `input(...)` / `emits(...)` / `vs_adapter(path)` annotations.
#[derive(Default)]
struct Annotations {
    input: Option<Vec<SchemaField>>,
    emits: Option<Vec<SchemaField>>,
    /// Path to a `fn(&mut dyn UdfContext, &str) -> Result<String, UdfError>`
    /// wired into the `virtual_schema_adapter_call` single-call vtable slot.
    vs_adapter: Option<Path>,
}

impl Parse for Annotations {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut annotations = Annotations::default();
        while !input.is_empty() {
            let section: Ident = input.parse()?;
            let content;
            syn::parenthesized!(content in input);
            match section.to_string().as_str() {
                "input" => {
                    annotations.input = Some(parse_schema_fields(&content)?);
                }
                "emits" => {
                    annotations.emits = Some(parse_schema_fields(&content)?);
                }
                "vs_adapter" => {
                    annotations.vs_adapter = Some(content.parse::<Path>()?);
                }
                other => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!(
                        "unknown annotation `{other}`, expected `input`, `emits`, or `vs_adapter`"
                    ),
                    ))
                }
            }
            // Allow a trailing comma between sections.
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(annotations)
    }
}

/// Parse a comma-separated `name: Type` field list from a parenthesized section.
fn parse_schema_fields(content: ParseStream) -> syn::Result<Vec<SchemaField>> {
    let fields = Punctuated::<SchemaField, Token![,]>::parse_terminated(content)?;
    Ok(fields.into_iter().collect())
}

/// Map a Rust type token to its ExaType JSON type name.
/// Returns an error (carrying the offending type's span) for unmappable types.
fn rust_type_to_exatype(ty: &Type) -> syn::Result<&'static str> {
    let name = type_token_string(ty);
    let mapped = match name.as_str() {
        "i32" => "Int32",
        "i64" => "Int64",
        "f64" => "Double",
        "f32" => "Double",
        "bool" => "Boolean",
        "String" => "String",
        "&str" | "str" => "String",
        "Decimal" => "Numeric",
        "NaiveDate" => "Date",
        "NaiveDateTime" => "Timestamp",
        _ => {
            return Err(syn::Error::new_spanned(
                ty,
                format!("unknown ExaType for {name}"),
            ))
        }
    };
    Ok(mapped)
}

/// Render a `Type` to a comparable string. References (`&str`) collapse to the
/// referent prefixed with `&` so `&str` maps cleanly.
fn type_token_string(ty: &Type) -> String {
    match ty {
        Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default(),
        Type::Reference(r) => format!("&{}", type_token_string(&r.elem)),
        _ => quote!(#ty).to_string().replace(' ', ""),
    }
}

/// Build the JSON schema string literal (NUL-terminated) for a field list.
fn schema_json(fields: &[SchemaField]) -> syn::Result<String> {
    let mut entries = Vec::with_capacity(fields.len());
    for f in fields {
        let exatype = rust_type_to_exatype(&f.ty)?;
        entries.push(format!(r#"{{"name":"{}","type":"{}"}}"#, f.name, exatype));
    }
    // The trailing NUL makes the byte slice a valid C string, since the vtable
    // exposes the pointer as `*const c_char`.
    Ok(format!("[{}]\0", entries.join(",")))
}

#[proc_macro_attribute]
pub fn exasol_udf(attr: TokenStream, item: TokenStream) -> TokenStream {
    let annotations = parse_macro_input!(attr as Annotations);
    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_name = &input_fn.sig.ident;

    let (input_schema_static, input_schema_ptr) =
        match build_schema_tokens(annotations.input.as_deref(), "__EXA_INPUT_SCHEMA") {
            Ok(parts) => parts,
            Err(e) => return e.to_compile_error().into(),
        };
    let (output_schema_static, output_schema_ptr) =
        match build_schema_tokens(annotations.emits.as_deref(), "__EXA_OUTPUT_SCHEMA") {
            Ok(parts) => parts,
            Err(e) => return e.to_compile_error().into(),
        };

    let (vs_adapter_shim, vs_adapter_slot) =
        build_vs_adapter_tokens(annotations.vs_adapter.as_ref());

    let expanded = quote! {
        #input_fn

        #input_schema_static
        #output_schema_static

        unsafe extern "C" fn __exa_run_shim(ctx_ptr: *mut ::std::ffi::c_void) -> i32 {
            // The closure captures `ctx_ptr` (a raw pointer), which is not
            // UnwindSafe. AssertUnwindSafe is sound here: nothing observable
            // is left in a broken state after a panic — the shim simply maps
            // the panic to error code 2 and returns.
            let result = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                // SAFETY: `ctx_ptr` is a thin pointer, but `&mut dyn UdfContext`
                // is a fat pointer (data + vtable), so it cannot be cast directly.
                // The ABI contract is therefore double-indirection: the host
                // passes `&mut (&mut dyn UdfContext)` erased to `*mut c_void`.
                // We restore the outer reference and dereference it to obtain the
                // fat trait-object reference. The host guarantees the pointer is
                // valid and outlives this call (see exasol_udf_sdk::abi docs).
                let ctx: &mut &mut dyn ::exasol_udf_sdk::context::UdfContext = unsafe {
                    &mut *(ctx_ptr as *mut &mut dyn ::exasol_udf_sdk::context::UdfContext)
                };
                #fn_name(*ctx)
            }));
            match result {
                ::std::result::Result::Ok(::std::result::Result::Ok(())) => 0,
                ::std::result::Result::Ok(::std::result::Result::Err(_)) => 1,
                ::std::result::Result::Err(_) => 2,
            }
        }

        unsafe extern "C" fn __exa_destroy_shim() {}

        #vs_adapter_shim

        #[used]
        static __EXA_VTABLE: ::exasol_udf_sdk::abi::ExaUdfVTable = ::exasol_udf_sdk::abi::ExaUdfVTable {
            abi_version: ::exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
            fingerprint: ::exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT.as_ptr() as *const ::std::ffi::c_char,
            run: __exa_run_shim,
            destroy: __exa_destroy_shim,
            default_output_columns: ::std::option::Option::None,
            virtual_schema_adapter_call: #vs_adapter_slot,
            generate_sql_for_import_spec: ::std::option::Option::None,
            generate_sql_for_export_spec: ::std::option::Option::None,
            annotated_input_schema: #input_schema_ptr,
            annotated_output_schema: #output_schema_ptr,
        };

        #[no_mangle]
        pub extern "C" fn __exa_udf_entry() -> *const ::exasol_udf_sdk::abi::ExaUdfVTable {
            &__EXA_VTABLE as *const _
        }
    };

    TokenStream::from(expanded)
}

/// Build the optional `virtual_schema_adapter_call` shim and the vtable slot
/// expression. When no `vs_adapter` annotation is present, no shim is emitted
/// and the slot is `None` (so the runtime replies MT_UNDEFINED_CALL, preserving
/// backward compatibility).
fn build_vs_adapter_tokens(path: Option<&Path>) -> (TokenStream2, TokenStream2) {
    match path {
        None => (quote! {}, quote! { ::std::option::Option::None }),
        Some(adapter_fn) => {
            let shim = quote! {
                unsafe extern "C" fn __exa_vs_adapter_shim(
                    ctx_ptr: *mut ::std::ffi::c_void,
                    json_arg: *const ::std::ffi::c_char,
                    result: *mut *mut ::std::ffi::c_char,
                ) -> i32 {
                    // Hand a Rust string to the runtime through a `malloc`-backed
                    // buffer; the runtime frees it with `free`, so allocation
                    // crosses the boundary entirely through the C allocator.
                    unsafe fn __exa_write_result(value: &str, out: *mut *mut ::std::ffi::c_char) {
                        extern "C" {
                            fn malloc(size: usize) -> *mut ::std::ffi::c_void;
                        }
                        // Replace interior NULs so the C string stays intact.
                        let sanitized = value.replace('\0', "\u{fffd}");
                        let len = sanitized.len() + 1;
                        let buf = unsafe { malloc(len) } as *mut u8;
                        if buf.is_null() {
                            unsafe { *out = ::std::ptr::null_mut() };
                            return;
                        }
                        unsafe {
                            ::std::ptr::copy_nonoverlapping(sanitized.as_ptr(), buf, sanitized.len());
                            *buf.add(sanitized.len()) = 0;
                            *out = buf as *mut ::std::ffi::c_char;
                        }
                    }

                    let outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                        // SAFETY: double-indirection ABI, identical to __exa_run_shim:
                        // the host passes `&mut (&mut dyn UdfContext)` erased to
                        // `*mut c_void` and guarantees it outlives this call.
                        let ctx: &mut &mut dyn ::exasol_udf_sdk::context::UdfContext = unsafe {
                            &mut *(ctx_ptr as *mut &mut dyn ::exasol_udf_sdk::context::UdfContext)
                        };
                        let json = if json_arg.is_null() {
                            ""
                        } else {
                            unsafe { ::std::ffi::CStr::from_ptr(json_arg) }
                                .to_str()
                                .unwrap_or("")
                        };
                        #adapter_fn(*ctx, json)
                    }));

                    match outcome {
                        ::std::result::Result::Ok(::std::result::Result::Ok(s)) => {
                            unsafe { __exa_write_result(&s, result) };
                            0
                        }
                        ::std::result::Result::Ok(::std::result::Result::Err(e)) => {
                            unsafe { __exa_write_result(&::std::string::ToString::to_string(&e), result) };
                            1
                        }
                        ::std::result::Result::Err(_) => {
                            unsafe { __exa_write_result("virtual_schema_adapter_call panicked", result) };
                            2
                        }
                    }
                }
            };
            let slot = quote! { ::std::option::Option::Some(__exa_vs_adapter_shim) };
            (shim, slot)
        }
    }
}

/// Build the optional schema `static` definition and the pointer expression for
/// the vtable field. When `fields` is `None`, no static is emitted and the
/// pointer is `null()`.
fn build_schema_tokens(
    fields: Option<&[SchemaField]>,
    const_name: &str,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    match fields {
        None => Ok((quote! {}, quote! { ::std::ptr::null() })),
        Some(fields) => {
            let json = schema_json(fields)?;
            let ident = syn::Ident::new(const_name, proc_macro2::Span::call_site());
            let static_def = quote! {
                static #ident: &str = #json;
            };
            let ptr = quote! { #ident.as_ptr() as *const ::std::ffi::c_char };
            Ok((static_def, ptr))
        }
    }
}
