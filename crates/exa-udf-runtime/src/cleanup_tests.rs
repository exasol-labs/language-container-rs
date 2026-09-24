use super::*;

#[test]
fn fold_keeps_the_original_error_first() {
    let folded = |outcome| fold(outcome, RuntimeError::Udf("cleanup broke".into())).unwrap_err();

    assert_eq!(
        folded(Err(RuntimeError::Udf("run broke".into()))).to_string(),
        "UDF error: run broke (cleanup also failed: cleanup broke)"
    );
    assert_eq!(
        folded(Err(RuntimeError::Unsupported("no %udf_object".into()))).to_string(),
        "UDF error: Unsupported feature: no %udf_object (cleanup also failed: cleanup broke)"
    );
    assert_eq!(folded(Ok(())).to_string(), "UDF error: cleanup broke");
}
