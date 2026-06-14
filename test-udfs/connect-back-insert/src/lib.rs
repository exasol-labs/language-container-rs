use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// SET UDF that inserts each input row's value into the pre-existing table
/// `cb_sink.cb_result` via a connect-back session, then emits a row-count of 1
/// per inserted row.
///
/// Transaction-isolation constraint (Exasol is Serializable): the connect-back
/// session (Part:44) runs in its OWN transaction, independent of the invoking
/// query's transaction (Part:40). To avoid a WAIT FOR COMMIT deadlock, this UDF
/// writes ONLY to `cb_sink.cb_result` — a table in a SEPARATE schema that the
/// invoking query does not read or lock — and performs NO DDL. The table and
/// its schema must be created and committed BEFORE the invoking query runs.
/// Writing to (or creating tables in) the invoking query's own schema would
/// force Part:40 into WAIT FOR COMMIT and trigger the deadlock detector.
///
/// Input is drained into a local buffer before opening the connect-back session
/// so the session is established only after the input phase is complete. The
/// named CONNECTION `CB_SELF` is fetched via the ZMQ MT_IMPORT exchange and used
/// as the connect-back address; the runtime opens a regular login session there.
#[exasol_udf]
pub fn connect_back_insert(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut vals: Vec<i64> = Vec::new();
    while ctx.next()? {
        let val = match ctx.get(0)? {
            Value::Int64(n) => *n,
            Value::Numeric(d) if d.scale == 0 => i64::try_from(d.unscaled)
                .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))?,
            _ => return Err(UdfError::Type("expected integer".into())),
        };
        vals.push(val);
    }

    let c = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&c)?;

    let placeholders = vals
        .iter()
        .map(|v| format!("({v})"))
        .collect::<Vec<_>>()
        .join(", ");
    if !placeholders.is_empty() {
        // The connect-back session uses autocommit (exarrow-rs default), so the
        // INSERT commits on its own. No explicit COMMIT — `execute_update("COMMIT")`
        // errors because there is no open transaction to commit.
        conn.execute(&format!(
            "INSERT INTO cb_sink.cb_result VALUES {placeholders}"
        ))?;
    }

    // Exasol represents BIGINT as PB_NUMERIC on the wire, so a BIGINT EMITS
    // column must be emitted as Value::Numeric. Emitting Value::Int64 puts the
    // data in the int64 block while the DB reads the (empty) string block for a
    // NUMERIC column, segfaulting its emit handler.
    for _ in &vals {
        ctx.emit(&[Value::Numeric(Decimal {
            unscaled: 1,
            scale: 0,
        })])?;
    }
    Ok(())
}
