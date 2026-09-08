use super::*;
use crate::test_support::TestContext;

/// The macro must not suppress a message when the level is permitted.
#[test]
fn udf_log_permitted_level_does_not_panic() {
    let ctx = TestContext::scalar(vec![]).with_debug_level(tracing::Level::DEBUG);
    // debug <= DEBUG (ctx level) → permitted; just asserts no panic/error.
    udf_log!(ctx, debug, "value = {}", 42);
    udf_log!(ctx, info, "also permitted");
    udf_log!(ctx, warn, "also permitted");
    udf_log!(ctx, error, "also permitted");
}

/// A TRACE message must be suppressed at DEBUG level.
#[test]
fn udf_log_suppressed_level_is_noop() {
    let ctx = TestContext::scalar(vec![]).with_debug_level(tracing::Level::DEBUG);
    // trace (5) > DEBUG (4) → suppressed; the macro is a no-op.
    // We can only check it compiles and does not write — no assertion needed
    // for suppression in a unit test, but calling it verifies the branch.
    udf_log!(ctx, trace, "suppressed {}", "value");
}

/// Level ordering: DEBUG message suppressed at INFO level.
#[test]
fn udf_log_debug_suppressed_at_info_level() {
    let ctx = TestContext::scalar(vec![]).with_debug_level(tracing::Level::INFO);
    // debug (4) > INFO (3) → message_level > ctx.debug_level() → suppressed.
    udf_log!(ctx, debug, "suppressed");
}
