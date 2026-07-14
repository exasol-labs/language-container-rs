use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, ItemFn, LitStr, Path, Token, Type, parse_macro_input};

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

/// The parsed `input(...)` / `emits(...)` / `vs_adapter(path)` / `name = "..."` annotations.
#[derive(Default)]
struct Annotations {
    input: Option<Vec<SchemaField>>,
    emits: Option<Vec<SchemaField>>,
    /// Path to a `fn(&mut dyn UdfContext, &str) -> Result<String, UdfError>`
    /// wired into the `virtual_schema_adapter_call` single-call vtable slot.
    vs_adapter: Option<Path>,
    /// Verbatim SQL name override; when absent the SQL name is derived from the
    /// Rust function identifier by uppercasing every ASCII character.
    name: Option<String>,
}

impl Parse for Annotations {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut annotations = Annotations::default();
        while !input.is_empty() {
            let section: Ident = input.parse()?;
            match section.to_string().as_str() {
                "name" => {
                    // `name = "literal"` — key-equals-value syntax, no parens.
                    input.parse::<Token![=]>()?;
                    let lit: LitStr = input.parse()?;
                    let value = lit.value();
                    if value.is_empty()
                        || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        return Err(syn::Error::new(
                            lit.span(),
                            "name must be non-empty and contain only ASCII alphanumeric characters and underscores",
                        ));
                    }
                    annotations.name = Some(value);
                }
                "input" => {
                    let content;
                    syn::parenthesized!(content in input);
                    annotations.input = Some(parse_schema_fields(&content)?);
                }
                "emits" => {
                    let content;
                    syn::parenthesized!(content in input);
                    annotations.emits = Some(parse_schema_fields(&content)?);
                }
                "vs_adapter" => {
                    let content;
                    syn::parenthesized!(content in input);
                    annotations.vs_adapter = Some(content.parse::<Path>()?);
                }
                other => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!(
                            "unknown annotation `{other}`, expected `name`, `input`, `emits`, or `vs_adapter`"
                        ),
                    ));
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

/// The output shape derived from a UDF function's return type.
enum OutputShape {
    /// `Result<(), UdfError>` — the UDF emits rows via `ctx.emit`.
    Emits,
    /// `Result<Option<T>, UdfError>` — the UDF returns one value per invocation.
    Returns,
}

/// Derive the output shape from the function return type: a `Result` whose `Ok`
/// type is `Option<T>` selects RETURNS; anything else (notably `Result<(), _>`)
/// selects EMITS.
fn derive_output_shape(output: &syn::ReturnType) -> OutputShape {
    if let syn::ReturnType::Type(_, ty) = output
        && let Some(ok_ty) = result_ok_type(ty)
        && is_option_type(ok_ty)
    {
        return OutputShape::Returns;
    }
    OutputShape::Emits
}

/// The last path segment of a `Type::Path`, if any.
fn last_path_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(p) => p.path.segments.last(),
        _ => None,
    }
}

/// The first type argument of a `Result<Ok, Err>` return type, if `ty` is a
/// `Result` path with angle-bracketed arguments.
fn result_ok_type(ty: &Type) -> Option<&Type> {
    let seg = last_path_segment(ty)?;
    if seg.ident != "Result" {
        return None;
    }
    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
        for arg in &args.args {
            if let syn::GenericArgument::Type(t) = arg {
                return Some(t);
            }
        }
    }
    None
}

/// Whether `ty` is an `Option<...>` path.
fn is_option_type(ty: &Type) -> bool {
    last_path_segment(ty)
        .map(|s| s.ident == "Option")
        .unwrap_or(false)
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
            ));
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
    let fn_ident = &input_fn.sig.ident;

    // Derive the SQL name: verbatim `name = "..."` override, else UPPER_SNAKE_CASE
    // from the Rust function identifier (ASCII uppercase; underscores preserved).
    let udf_name = match &annotations.name {
        Some(explicit) => explicit.clone(),
        None => fn_ident.to_string().to_uppercase(),
    };

    // Build all per-UDF suffixed identifiers so that multiple `#[exasol_udf]`
    // annotations with distinct names coexist in the same crate without symbol
    // collisions. Same-name annotations still collide at link time (desired).
    let input_schema_ident = format_ident!("__EXA_INPUT_SCHEMA_{udf_name}");
    let output_schema_ident = format_ident!("__EXA_OUTPUT_SCHEMA_{udf_name}");
    let write_c_string_ident = format_ident!("__exa_write_c_string_{udf_name}");
    let run_shim_ident = format_ident!("__exa_run_shim_{udf_name}");
    let destroy_shim_ident = format_ident!("__exa_destroy_shim_{udf_name}");
    let vtable_ident = format_ident!("__EXA_VTABLE_{udf_name}");
    let entry_ident = format_ident!("__exa_udf_entry_{udf_name}");

    let (input_schema_static, input_schema_ptr) =
        match build_schema_tokens(annotations.input.as_deref(), &input_schema_ident) {
            Ok(parts) => parts,
            Err(e) => return e.to_compile_error().into(),
        };
    let (output_schema_static, output_schema_ptr) =
        match build_schema_tokens(annotations.emits.as_deref(), &output_schema_ident) {
            Ok(parts) => parts,
            Err(e) => return e.to_compile_error().into(),
        };

    let (vs_adapter_shim, vs_adapter_slot) = build_vs_adapter_tokens(
        annotations.vs_adapter.as_ref(),
        &write_c_string_ident,
        &format_ident!("__exa_vs_adapter_shim_{udf_name}"),
    );

    // Derive the output shape from the return type. EMITS calls the UDF and lets
    // it emit; RETURNS threads the returned `Option<T>` through the `IntoValue`
    // conversion and the sanctioned `set_return` channel. Both branches produce
    // a `Result<(), UdfError>` so the run-shim's outer match is shared.
    let output_shape = derive_output_shape(&input_fn.sig.output);
    let (run_call_body, output_shape_expr) = match output_shape {
        OutputShape::Emits => (
            quote! { #fn_ident(*ctx) },
            quote! { ::exasol_udf_sdk::abi::OutputShape::Emits },
        ),
        OutputShape::Returns => (
            quote! {{
                let __exa_ret = #fn_ident(*ctx)?;
                // Map the inner value, not the whole `Option`, so `None` reaches
                // `set_return` as `None` (SQL NULL) rather than being collapsed to
                // `Value::Null` by `IntoValue for Option<T>` and then rewrapped in
                // `Some(..)` — `set_return`'s contract distinguishes the two.
                (*ctx).set_return(::std::option::Option::map(
                    __exa_ret,
                    ::exasol_udf_sdk::value::IntoValue::into_value,
                ))
            }},
            quote! { ::exasol_udf_sdk::abi::OutputShape::Returns },
        ),
    };

    let expanded = quote! {
        #input_fn

        #input_schema_static
        #output_schema_static

        // Hand a Rust string to the host through a `malloc`-backed buffer; the
        // host frees it with `libc::free`, so allocation and deallocation cross
        // the FFI boundary entirely through the C allocator — never mixing the
        // `.so`'s and host's separately-linked Rust global allocators (which
        // would be heap corruption for statically-linked musl `.so`s). Per-UDF
        // copy so distinct UDFs in the same crate do not share a helper symbol.
        unsafe fn #write_c_string_ident(value: &str, out: *mut *mut ::std::ffi::c_char) {
            unsafe extern "C" {
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

        unsafe extern "C" fn #run_shim_ident(
            ctx_ptr: *mut ::std::ffi::c_void,
            error_out: *mut *mut ::std::ffi::c_char,
        ) -> i32 {
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
                #run_call_body
            }));
            match result {
                ::std::result::Result::Ok(::std::result::Result::Ok(())) => 0,
                ::std::result::Result::Ok(::std::result::Result::Err(e)) => {
                    if !error_out.is_null() {
                        unsafe {
                            #write_c_string_ident(
                                &::std::string::ToString::to_string(&e),
                                error_out,
                            );
                        }
                    }
                    1
                }
                ::std::result::Result::Err(_) => 2,
            }
        }

        unsafe extern "C" fn #destroy_shim_ident() {}

        #vs_adapter_shim

        #[used]
        static #vtable_ident: ::exasol_udf_sdk::abi::ExaUdfVTable = ::exasol_udf_sdk::abi::ExaUdfVTable {
            abi_version: ::exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
            fingerprint: ::exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT.as_ptr() as *const ::std::ffi::c_char,
            run: #run_shim_ident,
            destroy: #destroy_shim_ident,
            default_output_columns: ::std::option::Option::None,
            virtual_schema_adapter_call: #vs_adapter_slot,
            generate_sql_for_import_spec: ::std::option::Option::None,
            generate_sql_for_export_spec: ::std::option::Option::None,
            annotated_input_schema: #input_schema_ptr,
            annotated_output_schema: #output_schema_ptr,
            output_shape: #output_shape_expr,
        };

        #[unsafe(no_mangle)]
        pub extern "C" fn #entry_ident() -> *const ::exasol_udf_sdk::abi::ExaUdfVTable {
            &#vtable_ident as *const _
        }
    };

    TokenStream::from(expanded)
}

/// Build the optional `virtual_schema_adapter_call` shim and the vtable slot
/// expression. When no `vs_adapter` annotation is present, no shim is emitted
/// and the slot is `None` (so the runtime replies MT_UNDEFINED_CALL, preserving
/// backward compatibility).
fn build_vs_adapter_tokens(
    path: Option<&Path>,
    write_c_string_ident: &proc_macro2::Ident,
    vs_adapter_shim_ident: &proc_macro2::Ident,
) -> (TokenStream2, TokenStream2) {
    match path {
        None => (quote! {}, quote! { ::std::option::Option::None }),
        Some(adapter_fn) => {
            let shim = quote! {
                unsafe extern "C" fn #vs_adapter_shim_ident(
                    ctx_ptr: *mut ::std::ffi::c_void,
                    json_arg: *const ::std::ffi::c_char,
                    result: *mut *mut ::std::ffi::c_char,
                ) -> i32 {
                    let outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                        // SAFETY: double-indirection ABI, identical to the run shim:
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
                            unsafe { #write_c_string_ident(&s, result) };
                            0
                        }
                        ::std::result::Result::Ok(::std::result::Result::Err(e)) => {
                            unsafe { #write_c_string_ident(&::std::string::ToString::to_string(&e), result) };
                            1
                        }
                        ::std::result::Result::Err(_) => {
                            unsafe { #write_c_string_ident("virtual_schema_adapter_call panicked", result) };
                            2
                        }
                    }
                }
            };
            let slot = quote! { ::std::option::Option::Some(#vs_adapter_shim_ident) };
            (shim, slot)
        }
    }
}

/// Build the optional schema `static` definition and the pointer expression for
/// the vtable field. When `fields` is `None`, no static is emitted and the
/// pointer is `null()`.
fn build_schema_tokens(
    fields: Option<&[SchemaField]>,
    ident: &proc_macro2::Ident,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    match fields {
        None => Ok((quote! {}, quote! { ::std::ptr::null() })),
        Some(fields) => {
            let json = schema_json(fields)?;
            let static_def = quote! {
                static #ident: &str = #json;
            };
            let ptr = quote! { #ident.as_ptr() as *const ::std::ffi::c_char };
            Ok((static_def, ptr))
        }
    }
}
