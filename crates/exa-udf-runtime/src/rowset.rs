use chrono::{NaiveDate, NaiveDateTime};
use exa_proto::ExascriptTableData;
use exa_zmq_protocol::{ColumnMeta, ExaType};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// Per-type column block width: each block holds exactly `n_rows` entries per
/// column (placeholder slots for NULL cells), so cell `(col, row)` lives at
/// `block_base + row`. The NULL bitmap is row-major: `row * n_cols + col`.
fn null_index(row: usize, col: usize, n_cols: usize) -> usize {
    row * n_cols + col
}

/// Materialised input rows from one proto `ExascriptTableData` batch.
///
/// Stored as a dense `rows[row][col]` matrix of `Value` for simplicity and
/// correctness; the per-type proto blocks are decoded once on construction.
pub struct InputRowSet {
    rows: Vec<Vec<Value>>,
    current_row: usize,
}

impl InputRowSet {
    /// Decode a proto batch into a row-major matrix of `Value`s.
    ///
    /// The proto packs each cell type into its own array, column by column,
    /// with one slot per row (including NULL cells). The NULL bitmap is
    /// row-major across all columns.
    pub fn from_proto(table: &ExascriptTableData, meta: &[ColumnMeta]) -> Self {
        let n_rows = table.rows as usize;
        let n_cols = meta.len();

        // Row-major within each type block: cells appear in (row, column) order,
        // so a column's value for row `r` is the `r`-th time that column's type
        // block is consumed while walking rows then columns. Per-type running
        // cursors advance only when a non-null cell of that type is read, mirroring
        // how `to_proto` packs (and how Exasol lays out emitted/input batches).
        let mut string_idx = 0usize;
        let mut bool_idx = 0usize;
        let mut int32_idx = 0usize;
        let mut int64_idx = 0usize;
        let mut double_idx = 0usize;

        let mut rows: Vec<Vec<Value>> = Vec::with_capacity(n_rows);
        for r in 0..n_rows {
            let mut row: Vec<Value> = Vec::with_capacity(n_cols);
            for (c, col) in meta.iter().enumerate() {
                let is_null = table
                    .data_nulls
                    .get(null_index(r, c, n_cols))
                    .copied()
                    .unwrap_or(false);
                if is_null {
                    // A NULL cell occupies no slot in its type block; do not
                    // advance the per-type cursor (see `to_proto`).
                    row.push(Value::Null);
                    continue;
                }
                let v = match &col.typ {
                    ExaType::Numeric { .. }
                    | ExaType::Date
                    | ExaType::Timestamp
                    | ExaType::TimestampTz
                    | ExaType::String { .. }
                    | ExaType::Char { .. }
                    | ExaType::Geometry
                    | ExaType::HashType
                    | ExaType::IntervalYearToMonth
                    | ExaType::IntervalDayToSecond => {
                        let s = table
                            .data_string
                            .get(string_idx)
                            .cloned()
                            .unwrap_or_default();
                        string_idx += 1;
                        decode_string_block(&col.typ, s)
                    }
                    ExaType::Boolean => {
                        let b = table.data_bool.get(bool_idx).copied().unwrap_or(false);
                        bool_idx += 1;
                        Value::Bool(b)
                    }
                    ExaType::Int32 => {
                        let i = table.data_int32.get(int32_idx).copied().unwrap_or(0);
                        int32_idx += 1;
                        Value::Int32(i)
                    }
                    ExaType::Int64 => {
                        let i = table.data_int64.get(int64_idx).copied().unwrap_or(0);
                        int64_idx += 1;
                        Value::Int64(i)
                    }
                    ExaType::Double => {
                        let f = table.data_double.get(double_idx).copied().unwrap_or(0.0);
                        double_idx += 1;
                        Value::Double(f)
                    }
                    ExaType::Unsupported => Value::Null,
                };
                row.push(v);
            }
            rows.push(row);
        }

        InputRowSet {
            rows,
            current_row: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Advance to the next row. Returns false when already on the last row.
    pub fn advance(&mut self) -> bool {
        if self.current_row + 1 < self.rows.len() {
            self.current_row += 1;
            true
        } else {
            false
        }
    }

    pub fn current_index(&self) -> usize {
        self.current_row
    }

    pub fn current_row(&self) -> &[Value] {
        &self.rows[self.current_row]
    }

    pub fn row(&self, idx: usize) -> Option<&[Value]> {
        self.rows.get(idx).map(|r| r.as_slice())
    }

    pub fn is_exhausted(&self) -> bool {
        self.current_row >= self.rows.len()
    }
}

/// Byte threshold at which the emit buffer requests a flush. Keeps the
/// serialised `MT_EMIT` payload well under the DB's per-message limits while
/// amortising the per-flush round-trip across many rows.
const EMIT_BUFFER_LIMIT_BYTES: usize = 4_000_000;

/// Conservative O(1) byte cost for one cell, approximating the width its
/// non-null value occupies in `to_proto`'s type block. NULL cells cost 0
/// because they take no type-block slot; the estimate slightly over-counts
/// (fixed widths for Numeric/Date/Timestamp) so the buffer flushes early rather
/// than late.
fn value_byte_cost(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Int32(_) => 4,
        Value::Int64(_) => 8,
        Value::Double(_) => 8,
        Value::String(s) => s.len(),
        // O(1) upper bound, no alloc: i128 renders in ≤39 digits + sign (≤40),
        // which also dominates the scale-padded form (sign + scale+1 + point).
        // Over-counts → flushes early, matching the conservative intent above.
        Value::Numeric(d) => 40 + d.scale as usize,
        Value::Date(_) => 10,
        Value::Timestamp(_) => 29,
    }
}

/// Accumulates emitted output rows, serialising to a proto batch on flush.
#[derive(Default)]
pub struct EmitBuffer {
    rows: Vec<Vec<Value>>,
    /// Running approximate serialised size of the buffered rows. Incremented in
    /// `push`, reset in `clear`; read by `should_flush`.
    byte_estimate: usize,
}

impl EmitBuffer {
    pub fn new() -> Self {
        EmitBuffer::default()
    }

    pub fn push(&mut self, values: Vec<Value>) {
        self.byte_estimate += values.iter().map(value_byte_cost).sum::<usize>();
        self.rows.push(values);
    }

    /// Whether the buffered rows have reached the byte threshold and should be
    /// flushed to the DB. A single oversized row trips this on its own push.
    pub fn should_flush(&self) -> bool {
        self.byte_estimate >= EMIT_BUFFER_LIMIT_BYTES
    }

    /// Serialise accumulated rows into an `ExascriptTableData`.
    ///
    /// Mirrors `InputRowSet::from_proto`: each type block is column-major and
    /// dense (one slot per row, including a placeholder for NULL cells) so the
    /// `block_base + row` indexing stays valid. The NULL bitmap is row-major.
    pub fn to_proto(&self, meta: &[ColumnMeta]) -> ExascriptTableData {
        let n_rows = self.rows.len();
        let n_cols = meta.len();

        let mut data_string: Vec<String> = Vec::new();
        let mut data_bool: Vec<bool> = Vec::new();
        let mut data_int32: Vec<i32> = Vec::new();
        let mut data_int64: Vec<i64> = Vec::new();
        let mut data_double: Vec<f64> = Vec::new();
        let mut data_nulls: Vec<bool> = vec![false; n_rows * n_cols];

        // Row-major within each type block: iterate rows, then columns, so a
        // column's cells appear in the block interleaved with the other columns
        // of the same type. This matches the layout Exasol's emit handler reads
        // (and that `from_proto` decodes); a column-major layout silently lands
        // later rows' values in the wrong column.
        //
        // Each value is packed into the block dictated by the declared column
        // type, not the runtime `Value` variant: a connect-back SELECT may
        // return a DECIMAL column as `Value::Int64`, but the EMITS column is
        // `ExaType::Numeric` (string block).
        for (r, row) in self.rows.iter().enumerate() {
            for (c, col) in meta.iter().enumerate() {
                let v = row.get(c).unwrap_or(&Value::Null);
                if matches!(v, Value::Null) {
                    // A NULL cell is recorded only in the bitmap; it does NOT
                    // occupy a slot in its type block. Exasol's reader consumes
                    // type-block entries only for non-null cells and consults the
                    // bitmap for nullness — a placeholder would shift every later
                    // cell of that type into the wrong column.
                    data_nulls[null_index(r, c, n_cols)] = true;
                    continue;
                }
                match &col.typ {
                    ExaType::Numeric { .. }
                    | ExaType::Date
                    | ExaType::Timestamp
                    | ExaType::TimestampTz
                    | ExaType::String { .. }
                    | ExaType::Char { .. }
                    | ExaType::Geometry
                    | ExaType::HashType
                    | ExaType::IntervalYearToMonth
                    | ExaType::IntervalDayToSecond => {
                        data_string.push(value_to_block_string(v));
                    }
                    ExaType::Boolean => data_bool.push(value_to_bool(v)),
                    ExaType::Int32 => data_int32.push(value_to_i64(v) as i32),
                    ExaType::Int64 => data_int64.push(value_to_i64(v)),
                    ExaType::Double => data_double.push(value_to_f64(v)),
                    ExaType::Unsupported => {}
                }
            }
        }

        ExascriptTableData {
            rows: n_rows as u64,
            rows_in_group: 0,
            data_string,
            data_nulls,
            data_bool,
            data_int32,
            data_int64,
            data_double,
            row_number: vec![],
        }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.byte_estimate = 0;
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Wire formats for the string-block temporal types. The DB serialises DATE as
/// `YYYY-MM-DD` and TIMESTAMP as `YYYY-MM-DD HH:MM:SS.ffffff` (space separator).
const DATE_FORMAT: &str = "%Y-%m-%d";
/// Parse format: `%.f` is optional fractional digits, tolerates both `HH:MM:SS` and `HH:MM:SS.ffffff`.
const TIMESTAMP_PARSE: &str = "%Y-%m-%d %H:%M:%S%.f";
/// Emit format: `%.9f` always emits exactly 9 fractional (nanosecond) digits.
/// The Exasol engine truncates the emitted value to the output column's declared
/// precision on receipt (`SWIGResultHandler::setTimestamp` parses `YYYY-MM-DD
/// HH24:MI:SS.FF9` then applies `trunc_to_fractional_seconds_precision(value,
/// m_types[col].prec)`), so emitting all 9 digits is lossless for every declared
/// precision; the old `%.6f` capped output at microseconds and lost precision for
/// `TIMESTAMP(7/8/9)`. This benefits only UDF-*generated* sub-microsecond values
/// (e.g. a wall-clock or connect-back source): the DB delivers input columns to
/// every UDF at microsecond precision (`SWIGTableData::getTimestamp` formats
/// `...FF6`), so an input→output round-trip is capped at microseconds regardless
/// of this emit format.
const TIMESTAMP_EMIT: &str = "%Y-%m-%d %H:%M:%S%.9f";
/// ISO-8601 `T`-separated fallback some sources emit for timestamps.
const TIMESTAMP_FORMAT_ISO: &str = "%Y-%m-%dT%H:%M:%S%.f";

/// Decode one non-null `data_string` cell into its typed `Value` per the column
/// type. NUMERIC/DATE/TIMESTAMP parse into their typed payloads; a parse failure
/// yields `Value::Null` so corrupt wire data stays decodable rather than
/// aborting the whole batch. Extended string-backed types pass through verbatim.
fn decode_string_block(typ: &ExaType, s: String) -> Value {
    match typ {
        ExaType::Numeric { .. } => match Decimal::try_from(s.as_str()) {
            Ok(d) => Value::Numeric(d),
            Err(_) => Value::Null,
        },
        ExaType::Date => match NaiveDate::parse_from_str(&s, DATE_FORMAT) {
            Ok(d) => Value::Date(d),
            Err(_) => Value::Null,
        },
        ExaType::Timestamp | ExaType::TimestampTz => {
            match NaiveDateTime::parse_from_str(&s, TIMESTAMP_PARSE)
                .or_else(|_| NaiveDateTime::parse_from_str(&s, TIMESTAMP_FORMAT_ISO))
            {
                Ok(ts) => Value::Timestamp(ts),
                Err(_) => Value::Null,
            }
        }
        _ => Value::String(s),
    }
}

/// Render a non-null `Value` as the text form for a string/numeric/temporal
/// block. Typed variants are serialised back to their wire form; numeric integer
/// and double variants are stringified so a DECIMAL EMITS column receiving a
/// `Value::Int64`/`Value::Double` from a connect-back SELECT still serialises.
fn value_to_block_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Numeric(d) => d.to_string(),
        Value::Date(d) => d.format(DATE_FORMAT).to_string(),
        Value::Timestamp(ts) => ts.format(TIMESTAMP_EMIT).to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
    }
}

/// Coerce a non-null `Value` to `i64` for an INT32/INT64 EMITS column.
fn value_to_i64(v: &Value) -> i64 {
    match v {
        Value::Int32(i) => *i as i64,
        Value::Int64(i) => *i,
        Value::Double(f) => *f as i64,
        Value::Numeric(d) => {
            let scaled = d.unscaled / 10i128.pow(d.scale as u32);
            i64::try_from(scaled).unwrap_or(0)
        }
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Bool(b) => *b as i64,
        _ => 0,
    }
}

/// Coerce a non-null `Value` to `f64` for a DOUBLE EMITS column.
fn value_to_f64(v: &Value) -> f64 {
    match v {
        Value::Double(f) => *f,
        Value::Int32(i) => *i as f64,
        Value::Int64(i) => *i as f64,
        Value::Numeric(d) => d.unscaled as f64 / 10f64.powi(d.scale as i32),
        Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Coerce a non-null `Value` to `bool` for a BOOLEAN EMITS column.
fn value_to_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int32(i) => *i != 0,
        Value::Int64(i) => *i != 0,
        Value::Numeric(d) => d.unscaled != 0,
        Value::String(s) => s == "true" || s == "TRUE" || s == "1",
        _ => false,
    }
}

/// Return the first non-loopback IPv4 address found on the local network
/// interfaces by walking the `getifaddrs` linked list.
///
/// Uses `libc::getifaddrs` because the UDF process is a normal Linux process
/// inside the Exasol container with full access to interface-enumeration
/// syscalls, and this approach works on single-node Docker (where the ZMQ
/// endpoint is `ipc://`) and multi-node TCP clusters alike.
#[cfg(feature = "connect-back")]
fn first_nonloopback_ipv4() -> Result<String, exasol_udf_sdk::error::UdfError> {
    use exasol_udf_sdk::error::UdfError;

    // Safety: `getifaddrs` is a POSIX syscall. `ifap` is only accessed inside
    // this function and freed before return. No Rust references alias the raw
    // pointer during traversal.
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    let rc = unsafe { libc::getifaddrs(&mut ifap) };
    if rc != 0 {
        return Err(UdfError::ConnectBack(format!(
            "getifaddrs failed with return code {rc}"
        )));
    }

    let mut result: Option<String> = None;

    // Walk the singly-linked list of interfaces.
    let mut ifa = ifap;
    while !ifa.is_null() {
        // Safety: `ifa` is a valid pointer produced by `getifaddrs`.
        let flags = unsafe { (*ifa).ifa_flags };
        let addr_ptr = unsafe { (*ifa).ifa_addr };

        // Skip interfaces that are not up or have no address.
        if flags & libc::IFF_UP as u32 != 0 && !addr_ptr.is_null() {
            // Safety: `addr_ptr` is non-null and valid per `getifaddrs` contract.
            let family = unsafe { (*addr_ptr).sa_family };
            if family == libc::AF_INET as libc::sa_family_t {
                // Safety: family is AF_INET so the pointer refers to a sockaddr_in.
                let sin = addr_ptr as *const libc::sockaddr_in;
                let s_addr = unsafe { (*sin).sin_addr.s_addr };

                // `s_addr` is in network byte order (big-endian). Converting to
                // host order and extracting the high byte gives the first IP
                // octet; 127 means loopback (127.0.0.0/8).
                let octets = u32::from_be(s_addr).to_be_bytes();
                if octets[0] != 127 {
                    result = Some(format!(
                        "{}.{}.{}.{}",
                        octets[0], octets[1], octets[2], octets[3]
                    ));
                    break;
                }
            }
        }

        // Safety: `ifa_next` is null-terminated per `getifaddrs` contract.
        ifa = unsafe { (*ifa).ifa_next };
    }

    // Always free the list regardless of outcome.
    // Safety: `ifap` was initialised by `getifaddrs` and has not been modified.
    unsafe { libc::freeifaddrs(ifap) };

    result.ok_or_else(|| UdfError::ConnectBack("no non-loopback IPv4 interface found".into()))
}

/// Bridges one materialised input batch and an emit buffer to the SDK's
/// `UdfContext`. The UDF advances through rows via `next` and reads the current
/// row via `get`; `emit` appends to the buffer.
///
/// `next` semantics: the first call positions on row 0 (returning whether any
/// row exists); subsequent calls advance. This lets both scalar and set UDFs
/// use the canonical `while ctx.next()? { ... }` loop while the dispatch layer
/// controls batch refills.
///
/// On-demand credential fetcher: given a CONNECTION name, sends MT_IMPORT to the
/// DB and returns the resulting `ConnInfo`. `Fn` (not `FnOnce`) because
/// `connection()` borrows `&self` and may be called repeatedly for different
/// named connections within a single UDF run.
#[cfg(feature = "connect-back")]
pub type ConnRequester<'a> =
    Box<dyn Fn(&str) -> Result<exa_zmq_protocol::ConnInfo, exasol_udf_sdk::error::UdfError> + 'a>;

/// Flushes the accumulated emit buffer to the DB mid-run. Receives `&mut
/// EmitBuffer` so it can serialise the rows (`to_proto`), send the `MT_EMIT`
/// exchange, and then `clear` the buffer. Feature-independent: mid-run flushing
/// is not gated on `connect-back`.
pub type EmitFlusher<'a> = Box<dyn FnMut(&mut EmitBuffer) -> Result<(), UdfError> + 'a>;

pub struct HostContextBridge<'a> {
    input: &'a mut InputRowSet,
    emit_buf: &'a mut EmitBuffer,
    input_cols: &'a [ColumnMeta],
    started: bool,
    /// Sends the buffered emit rows to the DB when the buffer crosses its byte
    /// threshold, keeping a single batch's output bounded. Invoked from `emit`.
    flusher: EmitFlusher<'a>,
    /// Last error captured from a UDF context method. Surfaced through
    /// `RuntimeError::Udf` so the full error appears in the SQL error. A `Cell`
    /// because `connection()` records errors through a shared `&self` borrow.
    last_error: std::cell::Cell<Option<String>>,
    /// Maximum memory the DB has allocated for this UDF invocation, in bytes.
    /// Sourced from `UdfMeta::maximal_memory_limit` at bridge construction time.
    memory_limit: u64,
    #[cfg(feature = "connect-back")]
    conn_requester: ConnRequester<'a>,
}

impl<'a> HostContextBridge<'a> {
    pub fn new(
        input: &'a mut InputRowSet,
        emit_buf: &'a mut EmitBuffer,
        input_cols: &'a [ColumnMeta],
        flusher: EmitFlusher<'a>,
        memory_limit: u64,
        #[cfg(feature = "connect-back")] conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            started: false,
            flusher,
            last_error: std::cell::Cell::new(None),
            memory_limit,
            #[cfg(feature = "connect-back")]
            conn_requester,
        }
    }

    /// Take the last error message captured from a UDF context method.
    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// Record an error message captured from a UDF context method. Available on
    /// a shared borrow because `connection()` is a `&self` method.
    #[cfg(feature = "connect-back")]
    fn record_error(&self, message: String) {
        self.last_error.set(Some(message));
    }

    /// Inject a credential fetcher so the bridge can be exercised without a live
    /// database. The supplied closure stands in for the on-demand MT_IMPORT
    /// exchange. Intended for tests.
    #[cfg(feature = "connect-back")]
    #[doc(hidden)]
    pub fn with_connection(
        input: &'a mut InputRowSet,
        emit_buf: &'a mut EmitBuffer,
        input_cols: &'a [ColumnMeta],
        flusher: EmitFlusher<'a>,
        memory_limit: u64,
        conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            started: false,
            flusher,
            last_error: std::cell::Cell::new(None),
            memory_limit,
            conn_requester,
        }
    }
}

/// Resolve a CONNECTION name to a [`ConnectionObject`] via the on-demand
/// credential fetcher. Shared by both context bridges.
#[cfg(feature = "connect-back")]
fn request_connection(
    requester: &ConnRequester,
    name: &str,
) -> Result<exasol_udf_sdk::connect_back::ConnectionObject, UdfError> {
    requester(name).map(|ci| exasol_udf_sdk::connect_back::ConnectionObject {
        kind: ci.kind,
        address: ci.address,
        user: ci.user,
        password: ci.password,
    })
}

/// Open a self-connection back to the DB from a resolved [`ConnectionObject`].
/// Shared by both context bridges.
#[cfg(feature = "connect-back")]
fn open_connect_back(
    conn: &exasol_udf_sdk::connect_back::ConnectionObject,
) -> Result<Box<dyn exasol_udf_sdk::connect_back::ExaConnection>, UdfError> {
    let info = exa_zmq_protocol::ConnInfo {
        kind: conn.kind.clone(),
        address: conn.address.clone(),
        user: conn.user.clone(),
        password: conn.password.clone(),
    };
    crate::connect_back::open_connection(&info)
        .map(|c| Box::new(c) as Box<dyn exasol_udf_sdk::connect_back::ExaConnection>)
}

impl UdfContext for HostContextBridge<'_> {
    fn num_columns(&self) -> usize {
        self.input_cols.len()
    }

    fn memory_limit(&self) -> u64 {
        self.memory_limit
    }

    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        self.input
            .current_row()
            .get(col)
            .ok_or_else(|| UdfError::Type(format!("column {col} out of range")))
    }

    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
        self.emit_buf.push(values.to_vec());
        if self.emit_buf.should_flush() {
            (self.flusher)(self.emit_buf)?;
        }
        Ok(())
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        if self.input.is_empty() {
            return Ok(false);
        }
        if !self.started {
            self.started = true;
            return Ok(true);
        }
        Ok(self.input.advance())
    }

    #[cfg(feature = "connect-back")]
    fn cluster_ip(&self) -> Result<String, UdfError> {
        let result = first_nonloopback_ipv4();
        if let Err(ref e) = result {
            self.record_error(e.to_string());
        }
        result
    }

    #[cfg(feature = "connect-back")]
    fn connection(
        &self,
        name: &str,
    ) -> Result<exasol_udf_sdk::connect_back::ConnectionObject, UdfError> {
        let result = request_connection(&self.conn_requester, name);
        if let Err(ref e) = result {
            self.record_error(e.to_string());
        }
        result
    }

    #[cfg(feature = "connect-back")]
    fn connect_back(
        &mut self,
        conn: &exasol_udf_sdk::connect_back::ConnectionObject,
    ) -> Result<Box<dyn exasol_udf_sdk::connect_back::ExaConnection>, UdfError> {
        let result = open_connect_back(conn);
        if let Err(ref e) = result {
            self.record_error(e.to_string());
        }
        result
    }
}

/// A `UdfContext` for single-call mode (e.g. the virtual-schema adapter call).
///
/// Single-call hooks receive no input rows and emit no output rows: the DB
/// exchanges one JSON request for one JSON response. The data methods therefore
/// return [`UdfError::Unimplemented`] — only credential resolution
/// (`connection`) and self-connections (`connect_back`) are meaningful, and
/// those reuse the same on-demand MT_IMPORT machinery as the data-UDF bridge.
pub struct SingleCallContext<'a> {
    /// Last error captured from a context method, surfaced through
    /// `RuntimeError::Udf`. A `Cell` because `connection()` borrows `&self`.
    last_error: std::cell::Cell<Option<String>>,
    #[cfg(feature = "connect-back")]
    conn_requester: ConnRequester<'a>,
    /// Anchors the `'a` lifetime when connect-back is disabled (the requester is
    /// the only `'a` user otherwise).
    #[cfg(not(feature = "connect-back"))]
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> SingleCallContext<'a> {
    pub fn new(#[cfg(feature = "connect-back")] conn_requester: ConnRequester<'a>) -> Self {
        SingleCallContext {
            last_error: std::cell::Cell::new(None),
            #[cfg(feature = "connect-back")]
            conn_requester,
            #[cfg(not(feature = "connect-back"))]
            _marker: std::marker::PhantomData,
        }
    }

    /// Take the last error message captured from a context method.
    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    #[cfg(feature = "connect-back")]
    fn record_error(&self, message: String) {
        self.last_error.set(Some(message));
    }
}

impl UdfContext for SingleCallContext<'_> {
    fn num_columns(&self) -> usize {
        0
    }

    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode has no input columns".into(),
        ))
    }

    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode does not emit rows".into(),
        ))
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode has no input rows".into(),
        ))
    }

    #[cfg(feature = "connect-back")]
    fn cluster_ip(&self) -> Result<String, UdfError> {
        let result = first_nonloopback_ipv4();
        if let Err(ref e) = result {
            self.record_error(e.to_string());
        }
        result
    }

    #[cfg(feature = "connect-back")]
    fn connection(
        &self,
        name: &str,
    ) -> Result<exasol_udf_sdk::connect_back::ConnectionObject, UdfError> {
        let result = request_connection(&self.conn_requester, name);
        if let Err(ref e) = result {
            self.record_error(e.to_string());
        }
        result
    }

    #[cfg(feature = "connect-back")]
    fn connect_back(
        &mut self,
        conn: &exasol_udf_sdk::connect_back::ConnectionObject,
    ) -> Result<Box<dyn exasol_udf_sdk::connect_back::ExaConnection>, UdfError> {
        let result = open_connect_back(conn);
        if let Err(ref e) = result {
            self.record_error(e.to_string());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, typ: ExaType) -> ColumnMeta {
        ColumnMeta {
            name: name.to_string(),
            typ,
            type_name: String::new(),
            size: None,
            precision: None,
            scale: None,
        }
    }

    /// Construct a bridge for the tests, supplying the connect-back arg only
    /// when the feature is enabled so the same call sites compile either way.
    fn make_bridge<'a>(
        input: &'a mut InputRowSet,
        emit: &'a mut EmitBuffer,
        cols: &'a [ColumnMeta],
    ) -> HostContextBridge<'a> {
        HostContextBridge::new(
            input,
            emit,
            cols,
            Box::new(|_buf: &mut EmitBuffer| Ok(())),
            0,
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        )
    }

    /// One batch, 2 rows, mixed types with a NULL cell. Verifies dense per-type
    /// block decoding and row-major NULL bitmap handling.
    fn mixed_batch() -> (ExascriptTableData, Vec<ColumnMeta>) {
        // Columns: [Int64, String, Double, Boolean]
        let meta = vec![
            col("a", ExaType::Int64),
            col("b", ExaType::String { size: None }),
            col("c", ExaType::Double),
            col("d", ExaType::Boolean),
        ];
        let n_rows = 2;
        let n_cols = 4;
        // row0: (10, "x", 1.5, true)   row1: (20, NULL-string, 2.5, false)
        let mut data_nulls = vec![false; n_rows * n_cols];
        // null at row1, col1 (string) -> index 1*4 + 1 = 5
        data_nulls[5] = true;
        let table = ExascriptTableData {
            rows: n_rows as u64,
            rows_in_group: 0,
            // string block (col1): row0="x", row1=placeholder ""
            data_string: vec!["x".into(), String::new()],
            data_nulls,
            data_bool: vec![true, false],
            data_int32: vec![],
            data_int64: vec![10, 20],
            data_double: vec![1.5, 2.5],
            row_number: vec![],
        };
        (table, meta)
    }

    #[test]
    fn bridge_materializes_input_rows() {
        let (table, meta) = mixed_batch();
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(rs.len(), 2);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::Int64(10),
                Value::String("x".into()),
                Value::Double(1.5),
                Value::Bool(true),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Int64(20),
                Value::Null,
                Value::Double(2.5),
                Value::Bool(false),
            ]
        );
    }

    #[test]
    fn bridge_typed_accessors() {
        let (table, meta) = mixed_batch();
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        let mut bridge = make_bridge(&mut rs, &mut emit, &meta);

        assert!(bridge.next().unwrap());
        assert_eq!(bridge.num_columns(), 4);
        assert_eq!(bridge.get(0).unwrap(), &Value::Int64(10));
        assert_eq!(bridge.get(1).unwrap(), &Value::String("x".into()));
        assert_eq!(bridge.get(3).unwrap(), &Value::Bool(true));
        assert!(matches!(bridge.get(99), Err(UdfError::Type(_))));

        assert!(bridge.next().unwrap());
        assert_eq!(bridge.get(0).unwrap(), &Value::Int64(20));
        assert_eq!(bridge.get(1).unwrap(), &Value::Null);

        assert!(!bridge.next().unwrap());
    }

    #[test]
    fn emit_buffer_roundtrips_through_proto() {
        let meta = vec![
            col("a", ExaType::Int64),
            col("b", ExaType::String { size: None }),
            col("c", ExaType::Double),
            col("d", ExaType::Boolean),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![
            Value::Int64(10),
            Value::String("x".into()),
            Value::Double(1.5),
            Value::Bool(true),
        ]);
        emit.push(vec![
            Value::Int64(20),
            Value::Null,
            Value::Double(2.5),
            Value::Bool(false),
        ]);

        let table = emit.to_proto(&meta);
        // Decoding the emitted batch back must reproduce the original rows,
        // proving from_proto/to_proto are symmetric (dense per-type blocks).
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::Int64(10),
                Value::String("x".into()),
                Value::Double(1.5),
                Value::Bool(true),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Int64(20),
                Value::Null,
                Value::Double(2.5),
                Value::Bool(false),
            ]
        );
    }

    #[test]
    fn emit_packs_by_declared_type_not_value_variant() {
        // A connect-back SELECT can return a DECIMAL column as Value::Int64, but
        // the EMITS column is ExaType::Numeric (string block). to_proto must
        // place it in the string block so the DB reads it from the right block.
        let meta = vec![
            col("region", ExaType::String { size: None }),
            col(
                "id",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::String("EU".into()), Value::Int64(1)]);
        emit.push(vec![Value::String("EU".into()), Value::Int64(2)]);

        let table = emit.to_proto(&meta);
        // Both columns are numeric/string -> the string block holds all cells in
        // row-major (row, column) order: row0 region,id then row1 region,id.
        assert_eq!(table.data_string, vec!["EU", "1", "EU", "2"]);
        assert!(table.data_int64.is_empty());

        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::String("EU".into()),
                Value::Numeric(Decimal::try_from("1").unwrap()),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::String("EU".into()),
                Value::Numeric(Decimal::try_from("2").unwrap()),
            ]
        );
    }

    #[test]
    fn emit_string_block_is_row_major_across_columns() {
        // Two same-type-block columns over two rows must interleave row-major in
        // data_string: row0(c0,c1) then row1(c0,c1). A column-major layout would
        // land row1's first cell where the DB expects row0's second column.
        let meta = vec![
            col(
                "a",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col("b", ExaType::String { size: None }),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![
            Value::Numeric(Decimal::try_from("100").unwrap()),
            Value::String("AAA".into()),
        ]);
        emit.push(vec![
            Value::Numeric(Decimal::try_from("200").unwrap()),
            Value::String("BBB".into()),
        ]);

        let table = emit.to_proto(&meta);
        assert_eq!(table.data_string, vec!["100", "AAA", "200", "BBB"]);

        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::Numeric(Decimal::try_from("100").unwrap()),
                Value::String("AAA".into()),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Numeric(Decimal::try_from("200").unwrap()),
                Value::String("BBB".into()),
            ]
        );
    }

    #[test]
    fn emit_null_cell_occupies_no_type_block_slot() {
        // A NULL numeric cell must not reserve a slot in the string block: the
        // bitmap marks it, and only the non-null "5" occupies the block. A
        // placeholder would shift "AAA"/"BBB" into the numeric column.
        let meta = vec![
            col(
                "id",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col("note", ExaType::String { size: None }),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::Null, Value::String("AAA".into())]);
        emit.push(vec![
            Value::Numeric(Decimal::try_from("5").unwrap()),
            Value::String("BBB".into()),
        ]);

        let table = emit.to_proto(&meta);
        // row0: id=NULL (skipped), note="AAA"; row1: id="5", note="BBB".
        assert_eq!(table.data_string, vec!["AAA", "5", "BBB"]);
        assert_eq!(table.data_nulls, vec![true, false, false, false]);

        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[Value::Null, Value::String("AAA".into())]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Numeric(Decimal::try_from("5").unwrap()),
                Value::String("BBB".into()),
            ]
        );
    }

    #[test]
    fn bridge_typed_getters_return_typed_options() {
        // A NUMERIC column must decode to Value::Numeric(Decimal) (carrying its
        // scale), a DATE to Value::Date(NaiveDate) and a TIMESTAMP to
        // Value::Timestamp(NaiveDateTime) — never a raw string. The fractional
        // timestamp exercises the %.f wire format on both decode and encode.
        let meta = vec![
            col(
                "amount",
                ExaType::Numeric {
                    precision: Some(10),
                    scale: Some(2),
                },
            ),
            col("d", ExaType::Date),
            col("ts", ExaType::Timestamp),
        ];
        let table = ExascriptTableData {
            rows: 1,
            rows_in_group: 0,
            data_string: vec![
                "12.34".into(),
                "2026-06-14".into(),
                "2026-06-14 09:30:15.250000".into(),
            ],
            data_nulls: vec![false, false, false],
            ..Default::default()
        };

        let rs = InputRowSet::from_proto(&table, &meta);
        let expected_date = NaiveDate::from_ymd_opt(2026, 6, 14).unwrap();
        let expected_ts = expected_date.and_hms_micro_opt(9, 30, 15, 250_000).unwrap();
        let decoded = rs.row(0).unwrap();
        assert_eq!(
            decoded[0],
            Value::Numeric(Decimal::try_from("12.34").unwrap())
        );
        assert_eq!(decoded[1], Value::Date(expected_date));
        assert_eq!(decoded[2], Value::Timestamp(expected_ts));

        // Round-trip: from_proto -> to_proto -> from_proto preserves typed values.
        let mut emit = EmitBuffer::new();
        emit.push(decoded.to_vec());
        let reproto = emit.to_proto(&meta);
        let reread = InputRowSet::from_proto(&reproto, &meta);
        assert_eq!(reread.row(0).unwrap(), decoded);
    }

    #[test]
    fn corrupt_string_block_value_decodes_to_null() {
        // A non-parseable NUMERIC/DATE cell must degrade to Value::Null rather
        // than aborting decode, so one corrupt cell cannot poison the batch.
        let meta = vec![
            col(
                "amount",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col("d", ExaType::Date),
        ];
        let table = ExascriptTableData {
            rows: 1,
            rows_in_group: 0,
            data_string: vec!["not-a-number".into(), "not-a-date".into()],
            data_nulls: vec![false, false],
            ..Default::default()
        };
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(rs.row(0).unwrap(), &[Value::Null, Value::Null]);
    }

    #[test]
    fn emit_buffer_byte_estimate_and_should_flush() {
        // Each row carries a 1000-byte string; the estimate grows by ~1000 per
        // push. Pushing rows until just below the 4 MB limit must keep
        // should_flush() false; one more push crosses it and flips it true.
        let row = || vec![Value::String("x".repeat(1000))];
        let mut emit = EmitBuffer::new();

        // Rows needed to first reach or exceed the limit.
        let rows_to_limit = EMIT_BUFFER_LIMIT_BYTES.div_ceil(1000);

        for _ in 0..(rows_to_limit - 1) {
            emit.push(row());
        }
        assert!(
            !emit.should_flush(),
            "buffer just below the limit must not request a flush"
        );

        emit.push(row());
        assert!(
            emit.should_flush(),
            "buffer at or above the limit must request a flush"
        );

        emit.clear();
        assert!(
            !emit.should_flush(),
            "clear() must reset the byte estimate so should_flush() is false"
        );
    }

    #[test]
    fn oversized_single_row_flushes_alone() {
        // A single row whose string exceeds the limit must trip should_flush()
        // after one push, so an oversized row is flushed on its own rather than
        // accumulating forever.
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::String("y".repeat(EMIT_BUFFER_LIMIT_BYTES + 1))]);
        assert!(
            emit.should_flush(),
            "a single oversized row must request a flush after one push"
        );
    }

    #[test]
    fn timestamp_emit_nanosecond_roundtrip() {
        // GIVEN a Timestamp value with sub-microsecond (nanosecond) precision.
        // 123456789 ns = 123456 µs + 789 ns; %.6f would truncate to 123456 µs.
        let ts = NaiveDate::from_ymd_opt(2026, 6, 14)
            .unwrap()
            .and_hms_nano_opt(9, 30, 15, 123_456_789)
            .unwrap();
        let meta = vec![col("ts", ExaType::Timestamp)];

        // WHEN serialised via EmitBuffer -> to_proto (uses value_to_block_string).
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::Timestamp(ts)]);
        let table = emit.to_proto(&meta);

        // THEN the emitted string contains exactly 9 fractional digits.
        let emitted_str = &table.data_string[0];
        assert!(
            emitted_str.ends_with(".123456789"),
            "expected 9-digit nanosecond fraction, got: {emitted_str}"
        );

        // AND it round-trips losslessly via from_proto.
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[Value::Timestamp(ts)],
            "nanosecond timestamp must survive to_proto -> from_proto round-trip"
        );
    }

    #[test]
    fn empty_batch_next_is_false() {
        let meta = vec![col("a", ExaType::Int64)];
        let table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        let mut bridge = make_bridge(&mut rs, &mut emit, &meta);
        assert!(!bridge.next().unwrap());
    }

    #[test]
    fn bridge_returns_memory_limit() {
        let meta = vec![col("a", ExaType::Int64)];
        let table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        let limit_bytes: u64 = 512 * 1024 * 1024;
        let bridge = HostContextBridge::new(
            &mut rs,
            &mut emit,
            &meta,
            Box::new(|_buf: &mut EmitBuffer| Ok(())),
            limit_bytes,
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        );
        assert_eq!(bridge.memory_limit(), limit_bytes);
    }
}
