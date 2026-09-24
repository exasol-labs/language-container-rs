use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use crate::rowset::CleanupContext;
use exa_zmq_protocol::UdfMeta;

/// Run the cleanup hook, if any, and fold its error into `outcome`, original
/// error first.
pub(crate) fn run_hook(
    udf: &LoadedUdf,
    meta: &UdfMeta,
    outcome: Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    let mut ctx = CleanupContext::from(meta);
    let cleanup = match udf.cleanup(&mut ctx) {
        None | Some(Ok(())) => return outcome,
        Some(Err(e)) => e.with_recorded_detail(ctx.take_last_error()),
    };
    fold(outcome, cleanup)
}

fn fold(outcome: Result<(), RuntimeError>, cleanup: RuntimeError) -> Result<(), RuntimeError> {
    let Err(original) = outcome else {
        return Err(cleanup);
    };
    Err(RuntimeError::Udf(format!(
        "{} (cleanup also failed: {})",
        message_of(original),
        message_of(cleanup)
    )))
}

fn message_of(error: RuntimeError) -> String {
    match error {
        RuntimeError::Udf(text) => text,
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
