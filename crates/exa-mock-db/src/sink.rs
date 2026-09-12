use exa_proto::ExascriptTableData;

/// `MT_EMIT` wire limit; the reference C++ container flushes after the row
/// that crosses it, and the database fills `MT_NEXT` batches to the same figure.
pub const EMIT_LIMIT_BYTES: usize = 4_000_000;

pub trait EmitSink {
    fn observe(&mut self, frame_len: usize, table: &ExascriptTableData);
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EmitCounters {
    pub messages: u64,
    pub rows: u64,
    pub bytes: u64,
    pub max_bytes: u64,
    pub over_limit: u64,
    pub with_row_number: u64,
}

impl EmitCounters {
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

#[derive(Debug, Default)]
pub struct EmitCollector {
    pub tables: Vec<ExascriptTableData>,
}

impl EmitCollector {
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
