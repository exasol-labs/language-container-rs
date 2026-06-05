use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

#[proc_macro_attribute]
pub fn exasol_udf(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);
    let fn_name = &input_fn.sig.ident;

    let expanded = quote! {
        #input_fn

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

        #[used]
        static __EXA_VTABLE: ::exasol_udf_sdk::abi::ExaUdfVTable = ::exasol_udf_sdk::abi::ExaUdfVTable {
            abi_version: ::exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION,
            fingerprint: ::exasol_udf_sdk::abi::EXA_SDK_FINGERPRINT.as_ptr() as *const ::std::ffi::c_char,
            run: __exa_run_shim,
            destroy: __exa_destroy_shim,
        };

        #[no_mangle]
        pub extern "C" fn __exa_udf_entry() -> *const ::exasol_udf_sdk::abi::ExaUdfVTable {
            &__EXA_VTABLE as *const _
        }
    };

    TokenStream::from(expanded)
}
