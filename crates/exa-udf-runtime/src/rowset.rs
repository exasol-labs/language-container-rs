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

        // Compute the per-column base offset within its type block.
        let mut string_idx = 0usize;
        let mut bool_idx = 0usize;
        let mut int32_idx = 0usize;
        let mut int64_idx = 0usize;
        let mut double_idx = 0usize;

        let mut col_offsets: Vec<(ExaType, usize)> = Vec::with_capacity(n_cols);
        for col in meta {
            match col.typ {
                ExaType::String | ExaType::Numeric | ExaType::Timestamp | ExaType::Date => {
                    col_offsets.push((col.typ.clone(), string_idx));
                    string_idx += n_rows;
                }
                ExaType::Boolean => {
                    col_offsets.push((ExaType::Boolean, bool_idx));
                    bool_idx += n_rows;
                }
                ExaType::Int32 => {
                    col_offsets.push((ExaType::Int32, int32_idx));
                    int32_idx += n_rows;
                }
                ExaType::Int64 => {
                    col_offsets.push((ExaType::Int64, int64_idx));
                    int64_idx += n_rows;
                }
                ExaType::Double => {
                    col_offsets.push((ExaType::Double, double_idx));
                    double_idx += n_rows;
                }
                ExaType::Unsupported => {
                    col_offsets.push((ExaType::Unsupported, 0));
                }
            }
        }

        let mut rows: Vec<Vec<Value>> = Vec::with_capacity(n_rows);
        for r in 0..n_rows {
            let mut row: Vec<Value> = Vec::with_capacity(n_cols);
            for (c, (typ, offset)) in col_offsets.iter().enumerate() {
                let is_null = table
                    .data_nulls
                    .get(null_index(r, c, n_cols))
                    .copied()
                    .unwrap_or(false);
                if is_null {
                    row.push(Value::Null);
                    continue;
                }
                let v = match typ {
                    ExaType::String | ExaType::Numeric | ExaType::Timestamp | ExaType::Date => {
                        let s = table
                            .data_string
                            .get(offset + r)
                            .cloned()
                            .unwrap_or_default();
                        match typ {
                            ExaType::Numeric => Value::Numeric(s),
                            ExaType::Timestamp => Value::Timestamp(s),
                            ExaType::Date => Value::Date(s),
                            _ => Value::String(s),
                        }
                    }
                    ExaType::Boolean => {
                        Value::Boolean(table.data_bool.get(offset + r).copied().unwrap_or(false))
                    }
                    ExaType::Int32 => {
                        Value::Int32(table.data_int32.get(offset + r).copied().unwrap_or(0))
                    }
                    ExaType::Int64 => {
                        Value::Int64(table.data_int64.get(offset + r).copied().unwrap_or(0))
                    }
                    ExaType::Double => {
                        Value::Double(table.data_double.get(offset + r).copied().unwrap_or(0.0))
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

        // Column-major within each type block: iterate columns, then rows.
        for (c, col) in meta.iter().enumerate() {
            for (r, row) in self.rows.iter().enumerate() {
                let v = row.get(c).unwrap_or(&Value::Null);
                if matches!(v, Value::Null) {
                    data_nulls[null_index(r, c, n_cols)] = true;
                    // Push a placeholder into the column's type block so the
                    // block width stays n_rows and indexing remains symmetric.
                    push_placeholder(
                        &col.typ,
                        &mut data_string,
                        &mut data_bool,
                        &mut data_int32,
                        &mut data_int64,
                        &mut data_double,
                    );
                    continue;
                }
                match v {
                    Value::String(s) | Value::Numeric(s) | Value::Timestamp(s) | Value::Date(s) => {
                        data_string.push(s.clone());
                    }
                    Value::Boolean(b) => data_bool.push(*b),
                    Value::Int32(i) => data_int32.push(*i),
                    Value::Int64(i) => data_int64.push(*i),
                    Value::Double(f) => data_double.push(*f),
                    Value::Null => unreachable!("null handled above"),
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

/// Push a type-appropriate placeholder for a NULL cell into the right block,
/// keeping every type block exactly `n_rows` wide per column.
fn push_placeholder(
    typ: &ExaType,
    data_string: &mut Vec<String>,
    data_bool: &mut Vec<bool>,
    data_int32: &mut Vec<i32>,
    data_int64: &mut Vec<i64>,
    data_double: &mut Vec<f64>,
) {
    match typ {
        ExaType::String | ExaType::Numeric | ExaType::Timestamp | ExaType::Date => {
            data_string.push(String::new());
        }
        ExaType::Boolean => data_bool.push(false),
        ExaType::Int32 => data_int32.push(0),
        ExaType::Int64 => data_int64.push(0),
        ExaType::Double => data_double.push(0.0),
        ExaType::Unsupported => {}
    }
}

/// Bridges one materialised input batch and an emit buffer to the SDK's
/// `UdfContext`. The UDF advances through rows via `next` and reads the current
/// row via `get`; `emit` appends to the buffer.
///
/// `next` semantics: the first call positions on row 0 (returning whether any
/// row exists); subsequent calls advance. This lets both scalar and set UDFs
/// use the canonical `while ctx.next()? { ... }` loop while the dispatch layer
/// controls batch refills.
pub struct HostContextBridge<'a> {
    input: &'a mut InputRowSet,
    emit_buf: &'a mut EmitBuffer,
    input_cols: &'a [ColumnMeta],
    started: bool,
    /// Last error captured from a UDF context method (e.g. `exa()`). Surfaced
    /// through `RuntimeError::Udf` so the full error appears in the SQL error.
    last_error: Option<String>,
    /// The lazily-opened default connect-back connection, cached for the life of
    /// the batch so repeated `exa()` calls reuse the same session.
    #[cfg(feature = "connect-back")]
    conn: Option<Box<dyn exasol_udf_sdk::connect_back::ExaConnection>>,
    /// Credentials surfaced during the handshake or a prior run-phase
    /// MT_IMPORT. Used to open `conn` on first `exa()` without sending a new
    /// request. `None` when no proactive credentials were received.
    #[cfg(feature = "connect-back")]
    conn_info: Option<exa_zmq_protocol::ConnInfo>,
    /// On-demand credential fetcher: called when `conn_info` is `None` and the
    /// UDF invokes `exa()`. Sends MT_IMPORT to the DB and returns the resulting
    /// `ConnInfo`. `FnOnce` because a single exchange is enough — the returned
    /// `ConnInfo` is used to open `conn`, which is cached for the batch.
    #[cfg(feature = "connect-back")]
    conn_requester: Option<
        Box<
            dyn FnOnce() -> Result<exa_zmq_protocol::ConnInfo, exasol_udf_sdk::error::UdfError>
                + 'a,
        >,
    >,
}

impl<'a> HostContextBridge<'a> {
    pub fn new(
        input: &'a mut InputRowSet,
        emit_buf: &'a mut EmitBuffer,
        input_cols: &'a [ColumnMeta],
        #[cfg(feature = "connect-back")] conn_info: Option<exa_zmq_protocol::ConnInfo>,
        #[cfg(feature = "connect-back")] conn_requester: Option<
            Box<
                dyn FnOnce() -> Result<exa_zmq_protocol::ConnInfo, exasol_udf_sdk::error::UdfError>
                    + 'a,
            >,
        >,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            started: false,
            last_error: None,
            #[cfg(feature = "connect-back")]
            conn: None,
            #[cfg(feature = "connect-back")]
            conn_info,
            #[cfg(feature = "connect-back")]
            conn_requester,
        }
    }

    /// Take the last error message captured from a UDF context method.
    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// Open the default connect-back connection. Tries proactively-received
    /// credentials first; falls back to an on-demand MT_IMPORT exchange.
    #[cfg(feature = "connect-back")]
    fn open_default_connection(&mut self) -> Result<(), exasol_udf_sdk::error::UdfError> {
        let info = if let Some(ci) = self.conn_info.take() {
            ci
        } else {
            let fetcher = self.conn_requester.take().ok_or_else(|| {
                exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no connection credentials available".into(),
                )
            })?;
            fetcher()?
        };
        self.conn = Some(Box::new(crate::connect_back::open_connection(&info)?));
        Ok(())
    }

    /// Inject a ready-made connection so the bridge can be exercised without a
    /// live database. Intended for tests.
    #[cfg(feature = "connect-back")]
    #[doc(hidden)]
    pub fn with_connection(
        input: &'a mut InputRowSet,
        emit_buf: &'a mut EmitBuffer,
        input_cols: &'a [ColumnMeta],
        conn: Box<dyn exasol_udf_sdk::connect_back::ExaConnection>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            started: false,
            last_error: None,
            conn: Some(conn),
            conn_info: None,
            conn_requester: None,
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
    fn exa(&mut self) -> Result<&mut dyn exasol_udf_sdk::connect_back::ExaConnection, UdfError> {
        if self.conn.is_none() {
            let result = self.open_default_connection();
            if let Err(ref e) = result {
                self.last_error = Some(e.to_string());
            }
            result?;
        }
        Ok(self.conn.as_deref_mut().expect("connection just opened"))
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
            None,
            #[cfg(feature = "connect-back")]
            None,
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
