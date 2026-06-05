use crate::error::RuntimeError;

/// JIT compilation (Option C) — unsupported in v1.
///
/// v1 only supports precompiled `.so` artifacts referenced via the
/// `%udf_object` script option.
pub fn compile_jit(_source: &str) -> Result<std::path::PathBuf, RuntimeError> {
    Err(RuntimeError::Unsupported(
        "JIT compilation (Option C) is not supported in v1; use %udf_object to load a precompiled .so".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jit_returns_unsupported() {
        let result = compile_jit("fn foo() {}");
        assert!(matches!(result, Err(RuntimeError::Unsupported(_))));
    }
}
