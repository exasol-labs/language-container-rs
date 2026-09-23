use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use crate::rowset::{CleanupContext, HandshakeMeta};
use exa_zmq_protocol::UdfMeta;

/// Run the UDF's cleanup hook, when it registered one, and fold its failure
/// into the dispatch `outcome`.
///
/// Called once per session after either dispatcher returns, so every dispatch
/// exit passes this one step. The hook receives a [`CleanupContext`] built from
/// the handshake metadata. When dispatch and the hook both failed, the single
/// resulting error names the original error first and the cleanup error
/// second, because the original error is the root cause.
pub(crate) fn run_hook(
    udf: &LoadedUdf,
    meta: &UdfMeta,
    outcome: Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    fold(outcome, invoke_hook(udf, meta))
}

fn invoke_hook(udf: &LoadedUdf, meta: &UdfMeta) -> Result<(), RuntimeError> {
    let mut ctx = CleanupContext::new(
        HandshakeMeta::from(meta),
        meta.input_iter(),
        meta.output_iter(),
    );
    let Some(result) = udf.cleanup(&mut ctx) else {
        return Ok(());
    };
    result.map_err(|e| e.with_recorded_detail(ctx.take_last_error()))
}

fn fold(
    outcome: Result<(), RuntimeError>,
    cleanup: Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    match (outcome, cleanup) {
        (outcome, Ok(())) => outcome,
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(original), Err(cleanup_error)) => Err(RuntimeError::Udf(format!(
            "{} (cleanup also failed: {})",
            message_of(&original),
            message_of(&cleanup_error)
        ))),
    }
}

fn message_of(error: &RuntimeError) -> String {
    match error {
        RuntimeError::Udf(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "cleanup_tests.rs"]
mod tests;
