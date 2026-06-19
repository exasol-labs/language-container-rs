use std::ffi::CStr;
use std::path::Path;

/// ABI version this binary was compiled against.
const RUNTIME_ABI_VERSION: u32 = exasol_udf_sdk::abi::EXA_UDF_ABI_VERSION;

/// SDK fingerprint baked in at compile time by build.rs.
const RUNTIME_FINGERPRINT: &str = env!("EXA_SDK_FINGERPRINT");

/// A minimal projection of the vtable — only the first two fields need reading.
#[repr(C)]
struct VTableProbe {
    abi_version: u32,
    fingerprint: *const std::ffi::c_char,
}

/// Validate a compiled UDF `.so` file.
pub fn run(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or_else(|| "Usage: cargo exasol-udf validate <path-to-so>".to_string())?;
    let so_path = Path::new(path);

    if !so_path.exists() {
        return Err(format!("file not found: '{}'", path));
    }

    let (abi_version, fingerprint) = load_vtable_fields(so_path)?;

    // ABI version check: the .so must not have a version newer than the runtime
    if abi_version > RUNTIME_ABI_VERSION {
        return Err(format!(
            "ABI version mismatch: .so has {}, runtime expects {}",
            abi_version, RUNTIME_ABI_VERSION
        ));
    }

    // SDK fingerprint check
    if fingerprint != RUNTIME_FINGERPRINT {
        return Err(format!(
            "SDK fingerprint mismatch: .so has '{}', runtime has '{}'",
            fingerprint, RUNTIME_FINGERPRINT
        ));
    }

    println!(
        "✓ ABI version: {}, fingerprint: {} — OK",
        abi_version, fingerprint
    );
    Ok(())
}

/// dlopen the `.so`, resolve `__exa_udf_entry`, and return `(abi_version, fingerprint)`.
fn load_vtable_fields(so_path: &Path) -> Result<(u32, String), String> {
    use libloading::Library;

    let lib = unsafe { Library::new(so_path) }
        .map_err(|e| format!("dlopen '{}' failed: {}", so_path.display(), e))?;

    let entry: libloading::Symbol<unsafe extern "C" fn() -> *const VTableProbe> =
        unsafe { lib.get(b"__exa_udf_entry\0") }.map_err(|_| {
            format!(
                "symbol __exa_udf_entry not found in '{}'",
                so_path.display()
            )
        })?;

    let vtable = unsafe { entry() };
    if vtable.is_null() {
        return Err("__exa_udf_entry returned null vtable pointer".to_string());
    }

    let abi_version = unsafe { (*vtable).abi_version };
    let fingerprint_ptr = unsafe { (*vtable).fingerprint };
    if fingerprint_ptr.is_null() {
        return Err("vtable fingerprint pointer is null".to_string());
    }

    let fingerprint = unsafe { CStr::from_ptr(fingerprint_ptr) }
        .to_str()
        .map_err(|e| format!("fingerprint is not valid UTF-8: {}", e))?
        .to_string();

    // Keep the library alive until we've read the data
    // (lib is dropped here, but the vtable points into .rodata of the .so —
    //  after dlclose the memory is unmapped, but we've already copied the
    //  fingerprint string above via .to_string())
    drop(lib);

    Ok((abi_version, fingerprint))
}
