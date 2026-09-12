use exa_proto::ExascriptTableData;

/// Wire limit of one `MT_EMIT` message, in bytes. The reference C++ container
/// flushes after every row that crosses it (`SWIG_MAX_VAR_DATASIZE`), and the
/// database fills each `MT_NEXT` batch until it passes the same figure.
pub const EMIT_LIMIT_BYTES: usize = 4_000_000;

/// Observer of the `MT_EMIT` messages a client sends during a run cycle.
///
/// The session acks each `MT_EMIT` as soon as it arrives, matching the pacing
/// of a database that acknowledges before it decodes, and calls `observe` once
/// per message after the timed window closes.
pub trait EmitSink {
    /// `frame_len` is the size of the whole request message on the wire;
    /// `table` is its decoded payload.
    fn observe(&mut self, frame_len: usize, table: &ExascriptTableData);
}

/// Counts and sizes `MT_EMIT` messages.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EmitCounters {
    /// Number of `MT_EMIT` messages.
    pub messages: u64,
    /// Total rows carried.
    pub rows: u64,
    /// Total bytes on the wire.
    pub bytes: u64,
    /// Largest single message, in bytes.
    pub max_bytes: u64,
    /// Messages larger than [`EMIT_LIMIT_BYTES`].
    pub over_limit: u64,
    /// Messages whose `row_number` field was populated, one entry per row.
    pub with_row_number: u64,
}

impl EmitCounters {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Mean bytes per message, 0 when nothing was observed.
    pub fn mean_bytes(&self) -> f64 {
        if self.messages == 0 {
            0.0
        } else {
            self.bytes as f64 / self.messages as f64
        }
    }
}

impl EmitSink for EmitCounters {
    fn observe(&mut self, frame_len: usize, table: &ExascriptTableData) {
        let len = frame_len as u64;
        self.messages += 1;
        self.rows += table.rows;
        self.bytes += len;
        self.max_bytes = self.max_bytes.max(len);
        if frame_len > EMIT_LIMIT_BYTES {
            self.over_limit += 1;
        }
        if table.row_number.len() as u64 == table.rows && table.rows > 0 {
            self.with_row_number += 1;
        }
    }
}

/// Keeps every emitted table, for tests that assert on values.
#[derive(Debug, Default)]
pub struct EmitCollector {
    pub tables: Vec<ExascriptTableData>,
}

impl EmitCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every `data_int64` cell across all tables, in emit order.
    pub fn int64_cells(&self) -> Vec<i64> {
        self.tables
            .iter()
            .flat_map(|t| t.data_int64.iter().copied())
            .collect()
    }

    pub fn rows(&self) -> u64 {
        self.tables.iter().map(|t| t.rows).sum()
    }
}

impl EmitSink for EmitCollector {
    fn observe(&mut self, _frame_len: usize, table: &ExascriptTableData) {
        self.tables.push(table.clone());
    }
}

#[cfg(test)]
#[path = "sink_tests.rs"]
mod tests;
