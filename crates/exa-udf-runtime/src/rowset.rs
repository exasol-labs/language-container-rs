use exa_proto::ExascriptTableData;
use exa_zmq_protocol::{ColumnMeta, ExaType};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

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
                let v = match col.typ {
                    ExaType::String | ExaType::Numeric | ExaType::Timestamp | ExaType::Date => {
                        let s = table
                            .data_string
                            .get(string_idx)
                            .cloned()
                            .unwrap_or_default();
                        string_idx += 1;
                        match col.typ {
                            ExaType::Numeric => Value::Numeric(s),
                            ExaType::Timestamp => Value::Timestamp(s),
                            ExaType::Date => Value::Date(s),
                            _ => Value::String(s),
                        }
                    }
                    ExaType::Boolean => {
                        let b = table.data_bool.get(bool_idx).copied().unwrap_or(false);
                        bool_idx += 1;
                        Value::Boolean(b)
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

/// Accumulates emitted output rows, serialising to a proto batch on flush.
#[derive(Default)]
pub struct EmitBuffer {
    rows: Vec<Vec<Value>>,
}

impl EmitBuffer {
    pub fn new() -> Self {
        EmitBuffer::default()
    }

    pub fn push(&mut self, values: Vec<Value>) {
        self.rows.push(values);
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
                match col.typ {
                    ExaType::String | ExaType::Numeric | ExaType::Timestamp | ExaType::Date => {
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
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Render a non-null `Value` as the text form for a string/numeric/temporal
/// block. Numeric, string and temporal variants pass through; numeric integer
/// and double variants are stringified so a DECIMAL EMITS column receiving a
/// `Value::Int64`/`Value::Double` from a connect-back SELECT still serialises.
fn value_to_block_string(v: &Value) -> String {
    match v {
        Value::String(s) | Value::Numeric(s) | Value::Timestamp(s) | Value::Date(s) => s.clone(),
        Value::Int32(i) => i.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Null => String::new(),
    }
}

/// Coerce a non-null `Value` to `i64` for an INT32/INT64 EMITS column.
fn value_to_i64(v: &Value) -> i64 {
    match v {
        Value::Int32(i) => *i as i64,
        Value::Int64(i) => *i,
        Value::Double(f) => *f as i64,
        Value::Numeric(s) | Value::String(s) => s.parse().unwrap_or(0),
        Value::Boolean(b) => *b as i64,
        _ => 0,
    }
}

/// Coerce a non-null `Value` to `f64` for a DOUBLE EMITS column.
fn value_to_f64(v: &Value) -> f64 {
    match v {
        Value::Double(f) => *f,
        Value::Int32(i) => *i as f64,
        Value::Int64(i) => *i as f64,
        Value::Numeric(s) | Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Coerce a non-null `Value` to `bool` for a BOOLEAN EMITS column.
fn value_to_bool(v: &Value) -> bool {
    match v {
        Value::Boolean(b) => *b,
        Value::Int32(i) => *i != 0,
        Value::Int64(i) => *i != 0,
        Value::String(s) | Value::Numeric(s) => s == "true" || s == "TRUE" || s == "1",
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

pub struct HostContextBridge<'a> {
    input: &'a mut InputRowSet,
    emit_buf: &'a mut EmitBuffer,
    input_cols: &'a [ColumnMeta],
    started: bool,
    /// Last error captured from a UDF context method. Surfaced through
    /// `RuntimeError::Udf` so the full error appears in the SQL error. A `Cell`
    /// because `connection()` records errors through a shared `&self` borrow.
    last_error: std::cell::Cell<Option<String>>,
    #[cfg(feature = "connect-back")]
    conn_requester: ConnRequester<'a>,
}

impl<'a> HostContextBridge<'a> {
    pub fn new(
        input: &'a mut InputRowSet,
        emit_buf: &'a mut EmitBuffer,
        input_cols: &'a [ColumnMeta],
        #[cfg(feature = "connect-back")] conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            started: false,
            last_error: std::cell::Cell::new(None),
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
        conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            started: false,
            last_error: std::cell::Cell::new(None),
            conn_requester,
        }
    }
}

impl UdfContext for HostContextBridge<'_> {
    fn num_columns(&self) -> usize {
        self.input_cols.len()
    }

    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        self.input
            .current_row()
            .get(col)
            .ok_or_else(|| UdfError::Type(format!("column {col} out of range")))
    }

    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
        self.emit_buf.push(values.to_vec());
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
        let result =
            (self.conn_requester)(name).map(|ci| exasol_udf_sdk::connect_back::ConnectionObject {
                kind: ci.kind,
                address: ci.address,
                user: ci.user,
                password: ci.password,
            });
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
        let info = exa_zmq_protocol::ConnInfo {
            kind: conn.kind.clone(),
            address: conn.address.clone(),
            user: conn.user.clone(),
            password: conn.password.clone(),
        };
        let result = crate::connect_back::open_connection(&info)
            .map(|c| Box::new(c) as Box<dyn exasol_udf_sdk::connect_back::ExaConnection>);
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
        let result =
            (self.conn_requester)(name).map(|ci| exasol_udf_sdk::connect_back::ConnectionObject {
                kind: ci.kind,
                address: ci.address,
                user: ci.user,
                password: ci.password,
            });
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
        let info = exa_zmq_protocol::ConnInfo {
            kind: conn.kind.clone(),
            address: conn.address.clone(),
            user: conn.user.clone(),
            password: conn.password.clone(),
        };
        let result = crate::connect_back::open_connection(&info)
            .map(|c| Box::new(c) as Box<dyn exasol_udf_sdk::connect_back::ExaConnection>);
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
            col("b", ExaType::String),
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
                Value::Boolean(true),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Int64(20),
                Value::Null,
                Value::Double(2.5),
                Value::Boolean(false),
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
        assert_eq!(bridge.get(3).unwrap(), &Value::Boolean(true));
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
            col("b", ExaType::String),
            col("c", ExaType::Double),
            col("d", ExaType::Boolean),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![
            Value::Int64(10),
            Value::String("x".into()),
            Value::Double(1.5),
            Value::Boolean(true),
        ]);
        emit.push(vec![
            Value::Int64(20),
            Value::Null,
            Value::Double(2.5),
            Value::Boolean(false),
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
                Value::Boolean(true),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Int64(20),
                Value::Null,
                Value::Double(2.5),
                Value::Boolean(false),
            ]
        );
    }

    #[test]
    fn emit_packs_by_declared_type_not_value_variant() {
        // A connect-back SELECT can return a DECIMAL column as Value::Int64, but
        // the EMITS column is ExaType::Numeric (string block). to_proto must
        // place it in the string block so the DB reads it from the right block.
        let meta = vec![col("region", ExaType::String), col("id", ExaType::Numeric)];
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
            &[Value::String("EU".into()), Value::Numeric("1".into())]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[Value::String("EU".into()), Value::Numeric("2".into())]
        );
    }

    #[test]
    fn emit_string_block_is_row_major_across_columns() {
        // Two same-type-block columns over two rows must interleave row-major in
        // data_string: row0(c0,c1) then row1(c0,c1). A column-major layout would
        // land row1's first cell where the DB expects row0's second column.
        let meta = vec![col("a", ExaType::Numeric), col("b", ExaType::String)];
        let mut emit = EmitBuffer::new();
        emit.push(vec![
            Value::Numeric("100".into()),
            Value::String("AAA".into()),
        ]);
        emit.push(vec![
            Value::Numeric("200".into()),
            Value::String("BBB".into()),
        ]);

        let table = emit.to_proto(&meta);
        assert_eq!(table.data_string, vec!["100", "AAA", "200", "BBB"]);

        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[Value::Numeric("100".into()), Value::String("AAA".into())]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[Value::Numeric("200".into()), Value::String("BBB".into())]
        );
    }

    #[test]
    fn emit_null_cell_occupies_no_type_block_slot() {
        // A NULL numeric cell must not reserve a slot in the string block: the
        // bitmap marks it, and only the non-null "5" occupies the block. A
        // placeholder would shift "AAA"/"BBB" into the numeric column.
        let meta = vec![col("id", ExaType::Numeric), col("note", ExaType::String)];
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::Null, Value::String("AAA".into())]);
        emit.push(vec![
            Value::Numeric("5".into()),
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
            &[Value::Numeric("5".into()), Value::String("BBB".into())]
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
}
