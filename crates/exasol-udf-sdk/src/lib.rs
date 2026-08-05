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
#[path = "lib_tests.rs"]
mod tests;
