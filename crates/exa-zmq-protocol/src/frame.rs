use exa_proto::{ExascriptRequest, ExascriptTableData, MessageType};
use prost::Message;
use prost::encoding::{self as enc, WireType, encode_key, encode_varint, encoded_len_varint};

/// A request the transport can put on the wire.
pub trait Frame {
    fn message_type(&self) -> i32;
    fn encode_frame(&self) -> Vec<u8>;
}

impl Frame for ExascriptRequest {
    fn message_type(&self) -> i32 {
        self.r#type
    }

    fn encode_frame(&self) -> Vec<u8> {
        self.encode_to_vec()
    }
}

/// `MT_EMIT` output blocks. The string block is kept in its wire form (key,
/// length, bytes per cell), so a flush copies it into the frame in one piece.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EmitTable {
    pub rows: u64,
    strings: Vec<u8>,
    pub nulls: Vec<bool>,
    pub bools: Vec<bool>,
    pub int32: Vec<i32>,
    pub int64: Vec<i64>,
    pub doubles: Vec<f64>,
    pub row_numbers: Vec<u64>,
}

impl EmitTable {
    pub fn push_string(&mut self, s: &[u8]) {
        encode_key(2, WireType::LengthDelimited, &mut self.strings);
        encode_varint(s.len() as u64, &mut self.strings);
        self.strings.extend_from_slice(s);
    }

    /// Append the cell `write` produces. Its length prefix is reserved as one
    /// byte and widened only for a cell of 128 bytes or more.
    pub fn push_string_with(&mut self, write: impl FnOnce(&mut Vec<u8>)) {
        encode_key(2, WireType::LengthDelimited, &mut self.strings);
        let at = self.strings.len();
        self.strings.push(0);
        write(&mut self.strings);
        let len = self.strings.len() - at - 1;
        if len < 0x80 {
            self.strings[at] = len as u8;
        } else {
            let mut prefix = Vec::with_capacity(10);
            encode_varint(len as u64, &mut prefix);
            self.strings.splice(at..=at, prefix);
        }
    }

    /// Empty every block, releasing a string block that grew past
    /// `max_retained` bytes.
    pub fn clear(&mut self, max_retained: usize) {
        self.rows = 0;
        if self.strings.capacity() > max_retained {
            self.strings = Vec::new();
        }
        self.strings.clear();
        self.nulls.clear();
        self.bools.clear();
        self.int32.clear();
        self.int64.clear();
        self.doubles.clear();
        self.row_numbers.clear();
    }

    fn string_cells(&self) -> impl Iterator<Item = &[u8]> {
        let mut rest = self.strings.as_slice();
        std::iter::from_fn(move || {
            let (_, tail) = rest.split_first()?;
            rest = tail;
            let len = enc::decode_varint(&mut rest).expect("string block length") as usize;
            let (cell, tail) = rest.split_at(len);
            rest = tail;
            Some(cell)
        })
    }

    pub fn to_proto(&self) -> ExascriptTableData {
        ExascriptTableData {
            rows: self.rows,
            rows_in_group: 0,
            data_string: self
                .string_cells()
                .map(bytes::Bytes::copy_from_slice)
                .collect(),
            data_nulls: self.nulls.clone(),
            data_bool: self.bools.clone(),
            data_int32: self.int32.clone(),
            data_int64: self.int64.clone(),
            data_double: self.doubles.clone(),
            row_number: self.row_numbers.clone(),
        }
    }

    fn encoded_len(&self) -> usize {
        enc::uint64::encoded_len(1, &self.rows)
            + self.strings.len()
            + enc::bool::encoded_len_packed(3, &self.nulls)
            + enc::bool::encoded_len_packed(4, &self.bools)
            + enc::int32::encoded_len_packed(5, &self.int32)
            + enc::int64::encoded_len_packed(6, &self.int64)
            + enc::double::encoded_len_packed(7, &self.doubles)
            + enc::uint64::encoded_len(8, &0)
            + enc::uint64::encoded_len_packed(9, &self.row_numbers)
    }

    /// The fields in tag order, as prost encodes `exascript_table_data`.
    fn encode_raw(&self, buf: &mut Vec<u8>) {
        enc::uint64::encode(1, &self.rows, buf);
        buf.extend_from_slice(&self.strings);
        enc::bool::encode_packed(3, &self.nulls, buf);
        enc::bool::encode_packed(4, &self.bools, buf);
        enc::int32::encode_packed(5, &self.int32, buf);
        enc::int64::encode_packed(6, &self.int64, buf);
        enc::double::encode_packed(7, &self.doubles, buf);
        enc::uint64::encode(8, &0, buf);
        enc::uint64::encode_packed(9, &self.row_numbers, buf);
    }
}

/// An `MT_EMIT` request, encoded byte-identically to the equivalent
/// `ExascriptRequest`.
pub struct EmitRequest<'a> {
    pub connection_id: u64,
    pub table: &'a EmitTable,
}

impl Frame for EmitRequest<'_> {
    fn message_type(&self) -> i32 {
        MessageType::MtEmit as i32
    }

    fn encode_frame(&self) -> Vec<u8> {
        let mt = self.message_type();
        let table_len = self.table.encoded_len();
        let emit_len = 1 + encoded_len_varint(table_len as u64) + table_len;
        let mut buf = Vec::with_capacity(
            enc::int32::encoded_len(1, &mt)
                + enc::uint64::encoded_len(2, &self.connection_id)
                + 1
                + encoded_len_varint(emit_len as u64)
                + emit_len,
        );
        enc::int32::encode(1, &mt, &mut buf);
        enc::uint64::encode(2, &self.connection_id, &mut buf);
        encode_key(7, WireType::LengthDelimited, &mut buf);
        encode_varint(emit_len as u64, &mut buf);
        encode_key(2, WireType::LengthDelimited, &mut buf);
        encode_varint(table_len as u64, &mut buf);
        self.table.encode_raw(&mut buf);
        buf
    }
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
