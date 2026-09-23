use super::*;

fn udf_error(text: &str) -> RuntimeError {
    RuntimeError::Udf(text.to_string())
}

#[test]
fn fold_keeps_the_original_error_first() {
    let folded = fold(
        Err(udf_error(
            "UDF run returned error code 1: run failed on purpose",
        )),
        Err(udf_error(
            "UDF cleanup returned error code 1: cleanup failed on purpose",
        )),
    );

    assert_eq!(
        folded
            .expect_err("both failures must fail the session")
            .to_string(),
        "UDF error: UDF run returned error code 1: run failed on purpose \
         (cleanup also failed: UDF cleanup returned error code 1: cleanup failed on purpose)"
    );
}

#[test]
fn fold_keeps_a_non_udf_original_error_whole() {
    let folded = fold(
        Err(RuntimeError::Unsupported("no %udf_object".into())),
        Err(udf_error("UDF cleanup returned error code 2")),
    );

    assert_eq!(
        folded
            .expect_err("both failures must fail the session")
            .to_string(),
        "UDF error: Unsupported feature: no %udf_object \
         (cleanup also failed: UDF cleanup returned error code 2)"
    );
}

#[test]
fn fold_reports_the_cleanup_error_after_a_successful_dispatch() {
    let folded = fold(
        Ok(()),
        Err(udf_error("UDF cleanup returned error code 1: late")),
    );

    assert_eq!(
        folded
            .expect_err("a cleanup failure must fail the session")
            .to_string(),
        "UDF error: UDF cleanup returned error code 1: late"
    );
}

#[test]
fn fold_without_a_cleanup_error_keeps_the_dispatch_outcome() {
    assert!(fold(Ok(()), Ok(())).is_ok());
    assert_eq!(
        fold(Err(udf_error("run broke")), Ok(()))
            .expect_err("the dispatch error must survive")
            .to_string(),
        "UDF error: run broke"
    );
}
