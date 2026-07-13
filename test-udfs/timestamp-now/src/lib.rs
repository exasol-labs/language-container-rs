use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

/// Return the current local wall-clock time as a naive `TIMESTAMP`.
///
/// `chrono::Local` resolves the active zone from `TZ` + `/usr/share/zoneinfo`
/// (the IANA database), so the returned naive value reflects the container's
/// local time when `tzdata` is present, and falls back to UTC when it is not.
#[exasol_udf]
pub fn timestamp_now(_ctx: &mut dyn UdfContext) -> Result<Option<chrono::NaiveDateTime>, UdfError> {
    let now = chrono::Local::now().naive_local();
    Ok(Some(now))
}
