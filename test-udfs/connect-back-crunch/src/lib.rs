use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// SET UDF that number-crunches each input value (squares it) and writes the
/// pair `(v, v*v)` back into `it_rust.crunch_log` via a connect-back session,
/// then emits a row-count of 1 per written row.
///
/// Demonstrates that connect-back write-back works in the **same schema** as the
/// invoking query — provided Exasol's Serializable isolation is respected:
///
/// * `crunch_log` is created **and committed before** the invoking query runs,
///   so the connect-back session (its own, independent transaction) can see it
///   and the UDF performs **no DDL**.
/// * The invoking query reads a **different** table (`crunch_in`) than the one
///   the UDF writes (`crunch_log`), so there is no read-write conflict that
///   would force the invoking transaction into WAIT FOR COMMIT.
/// * The connect-back session autocommits (exarrow-rs default), so each INSERT
///   commits on its own — no explicit `COMMIT` (which would error: there is no
///   open transaction to commit).
///
/// Input is drained into a local buffer before opening the connect-back session
/// so the session is established only after the input phase is complete.
#[exasol_udf]
pub fn crunch_writeback(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
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

    // Number-crunch: square each input and write (v, v*v) into the pre-committed
    // table in the invoking query's own schema.
    let placeholders = vals
        .iter()
        .map(|v| format!("({v}, {})", v * v))
        .collect::<Vec<_>>()
        .join(", ");
    if !placeholders.is_empty() {
        conn.execute(&format!(
            "INSERT INTO it_rust.crunch_log VALUES {placeholders}"
        ))?;
    }

    // BIGINT EMITS columns travel as PB_NUMERIC (typed Decimal); emit Numeric,
    // not Int64, or the DB's emit handler reads the wrong block and crashes.
    for _ in &vals {
        ctx.emit(&[Value::Numeric(Decimal {
            unscaled: 1,
            scale: 0,
        })])?;
    }
    Ok(())
}
