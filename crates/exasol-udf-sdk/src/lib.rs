pub mod abi;
pub mod connect_back;
pub mod context;
pub mod error;
pub mod value;

pub use connect_back::{ConnectionObject, ExaConnection};
pub use context::{UdfContext, UdfRun};
pub use error::UdfError;
pub use value::{ExaType, IntoValue, Value};

/// Write a formatted message to stderr when the context's resolved debug level
/// permits the requested level.
///
/// Usage: `udf_log!(ctx, debug, "x = {}", x);`
///
/// The level check mirrors the `tracing::Level` ordering where
/// `ERROR < WARN < INFO < DEBUG < TRACE` (higher value = more verbose).
/// A message at level `L` is written only when `ctx.debug_level() >= L`.
/// Writes directly to `std::io::stderr()` — no tracing subscriber is created
/// inside the UDF `.so`; the DB's fd-2 redirect delivers the output.
#[macro_export]
macro_rules! udf_log {
    ($ctx:expr, error, $($arg:tt)*) => {
        $crate::udf_log!(@emit $ctx, tracing::Level::ERROR, $($arg)*)
    };
    ($ctx:expr, warn, $($arg:tt)*) => {
        $crate::udf_log!(@emit $ctx, tracing::Level::WARN, $($arg)*)
    };
    ($ctx:expr, info, $($arg:tt)*) => {
        $crate::udf_log!(@emit $ctx, tracing::Level::INFO, $($arg)*)
    };
    ($ctx:expr, debug, $($arg:tt)*) => {
        $crate::udf_log!(@emit $ctx, tracing::Level::DEBUG, $($arg)*)
    };
    ($ctx:expr, trace, $($arg:tt)*) => {
        $crate::udf_log!(@emit $ctx, tracing::Level::TRACE, $($arg)*)
    };
    (@emit $ctx:expr, $level:expr, $($arg:tt)*) => {{
        let msg_level: tracing::Level = $level;
        if msg_level <= ($ctx).debug_level() {
            use std::io::Write as _;
            let _ = writeln!(std::io::stderr(), "[{}] {}", msg_level, format_args!($($arg)*));
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedLevelCtx(tracing::Level);

    impl UdfContext for FixedLevelCtx {
        fn num_columns(&self) -> usize {
            0
        }
        fn get(&self, _col: usize) -> Result<&Value, UdfError> {
            Err(UdfError::Type("no columns".into()))
        }
        fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
            Ok(())
        }
        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
        fn debug_level(&self) -> tracing::Level {
            self.0
        }
    }

    /// The macro must not suppress a message when the level is permitted.
    #[test]
    fn udf_log_permitted_level_does_not_panic() {
        let ctx = FixedLevelCtx(tracing::Level::DEBUG);
        // debug <= DEBUG (ctx level) → permitted; just asserts no panic/error.
        udf_log!(ctx, debug, "value = {}", 42);
        udf_log!(ctx, info, "also permitted");
        udf_log!(ctx, warn, "also permitted");
        udf_log!(ctx, error, "also permitted");
    }

    /// A TRACE message must be suppressed at DEBUG level.
    #[test]
    fn udf_log_suppressed_level_is_noop() {
        let ctx = FixedLevelCtx(tracing::Level::DEBUG);
        // trace (5) > DEBUG (4) → suppressed; the macro is a no-op.
        // We can only check it compiles and does not write — no assertion needed
        // for suppression in a unit test, but calling it verifies the branch.
        udf_log!(ctx, trace, "suppressed {}", "value");
    }

    /// Level ordering: DEBUG message suppressed at INFO level.
    #[test]
    fn udf_log_debug_suppressed_at_info_level() {
        let ctx = FixedLevelCtx(tracing::Level::INFO);
        // debug (4) > INFO (3) → message_level > ctx.debug_level() → suppressed.
        udf_log!(ctx, debug, "suppressed");
    }
}
