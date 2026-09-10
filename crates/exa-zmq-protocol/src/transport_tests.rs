use super::*;

#[test]
fn retry_transient_retries_past_any_fixed_time_budget() {
    let mut calls = 0u32;
    let result = retry_transient(
        || {
            calls += 1;
            if calls <= 10_000 {
                Err(zmq::Error::EAGAIN)
            } else {
                Ok(calls)
            }
        },
        "test",
    );
    assert_eq!(result.unwrap(), 10_001);
}

#[test]
fn retry_transient_propagates_a_genuine_error_immediately() {
    let mut calls = 0u32;
    let result: Result<(), _> = retry_transient(
        || {
            calls += 1;
            Err(zmq::Error::EINVAL)
        },
        "test",
    );
    assert!(result.is_err());
    assert_eq!(calls, 1);
}
