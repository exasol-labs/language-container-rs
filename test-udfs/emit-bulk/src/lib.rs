use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

const DEFAULT_PAYLOAD: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // 98 chars + overhead

/// Build an ASCII payload of exactly `width` bytes, so a caller can size a
/// single emitted row precisely — e.g. to straddle or exceed the
/// `4_000_000`-byte `MT_EMIT` wire limit.
fn payload_of_width(width: usize) -> String {
    "A".repeat(width)
}

/// SCALAR EMITS fixture driving `EmitBuffer` boundary conditions. Column 0 is
/// the repeat count `n`; an optional column 1 is a row width in bytes. When
/// column 1 is absent or NULL, the payload is the original fixed-size filler
/// (preserving prior behavior); when present, each emitted row's string is
/// exactly that many bytes, letting IT drive both a single oversize row and
/// rows straddling the 4,000,000-byte threshold.
#[exasol_udf]
pub fn emit_bulk(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, width) = match ctx.next()? {
        false => return Ok(()),
        true => {
            let n = ctx
                .get_i64(0)?
                .ok_or_else(|| UdfError::Type("n must not be NULL".into()))?;
            let width = if ctx.input_column_count() > 1 {
                ctx.get_i64(1)?
            } else {
                None
            };
            (n, width)
        }
    };
    while ctx.next()? {} // drain remaining input rows
    let payload = match width {
        Some(w) if w >= 0 => payload_of_width(w as usize),
        _ => DEFAULT_PAYLOAD.to_string(),
    };
    for _ in 0..n {
        ctx.emit(vec![Value::String(payload.clone())])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exasol_udf_sdk::test_support::TestContext;

    #[test]
    fn default_payload_when_width_column_absent() {
        let mut ctx = TestContext::set(vec![vec![Value::Int64(2)]]);
        emit_bulk(&mut ctx).unwrap();
        assert_eq!(ctx.emitted().len(), 2);
        for row in ctx.emitted() {
            assert_eq!(row, &vec![Value::String(DEFAULT_PAYLOAD.to_string())]);
        }
    }

    #[test]
    fn default_payload_when_width_is_null() {
        let mut ctx = TestContext::set(vec![vec![Value::Int64(1), Value::Null]]);
        emit_bulk(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted(),
            vec![vec![Value::String(DEFAULT_PAYLOAD.to_string())]]
        );
    }

    #[test]
    fn explicit_width_controls_payload_size() {
        let mut ctx = TestContext::set(vec![vec![Value::Int64(1), Value::Int64(10)]]);
        emit_bulk(&mut ctx).unwrap();
        assert_eq!(ctx.emitted().len(), 1);
        match &ctx.emitted()[0][0] {
            Value::String(s) => assert_eq!(s.len(), 10),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn width_can_exceed_emit_buffer_threshold() {
        let over_threshold = 4_000_001;
        let mut ctx = TestContext::set(vec![vec![Value::Int64(1), Value::Int64(over_threshold)]]);
        emit_bulk(&mut ctx).unwrap();
        assert_eq!(ctx.emitted().len(), 1);
        match &ctx.emitted()[0][0] {
            Value::String(s) => assert_eq!(s.len(), over_threshold as usize),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn zero_count_emits_nothing() {
        let mut ctx = TestContext::set(vec![vec![Value::Int64(0)]]);
        emit_bulk(&mut ctx).unwrap();
        assert!(ctx.emitted().is_empty());
    }
}
