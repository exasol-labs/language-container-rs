use super::*;
use exasol_udf_sdk::test_support::TestContext;

#[test]
fn reports_the_group_size_and_the_rows_it_iterated() {
    let mut ctx = TestContext::set(vec![
        vec![Value::Int64(7), Value::Int64(10)],
        vec![Value::Int64(7), Value::Int64(20)],
        vec![Value::Int64(7), Value::Int64(30)],
    ])
    .with_rows_in_group(3);

    rows_in_group(&mut ctx).unwrap();

    assert_eq!(
        ctx.emitted(),
        &[vec![Value::Int64(7), Value::Int64(3), Value::Int64(3)]],
        "must emit the group key alongside the reported count and the number of rows iterated"
    );
}

#[test]
fn reports_the_declared_count_for_an_empty_group() {
    let mut ctx = TestContext::set(vec![]).with_rows_in_group(0);

    rows_in_group(&mut ctx).unwrap();

    assert_eq!(
        ctx.emitted(),
        &[vec![Value::Int64(0), Value::Int64(0), Value::Int64(0)]],
        "rows_in_group is read before the first next(), not derived from the number of rows iterated"
    );
}
