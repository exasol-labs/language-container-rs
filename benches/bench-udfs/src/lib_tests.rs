use super::*;
use arrow::array::{Array, Date32Array, Decimal128Array, Int64Array, StringArray};
use exasol_udf_sdk::test_support::{EmitPolicy, TestContext};

// ---------------------------------------------------------------------------
// Test double capturing `emit_record_batch_ipc`, which `TestContext` lacks.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct BatchCapture {
    rows: Vec<Vec<Value>>,
    cursor: Option<usize>,
    scalar: bool,
    batches: Vec<RecordBatch>,
    emitted_rows: Vec<Vec<Value>>,
}

impl BatchCapture {
    fn set(rows: Vec<Vec<Value>>) -> Self {
        BatchCapture {
            rows,
            ..Default::default()
        }
    }
    fn scalar(row: Vec<Value>) -> Self {
        BatchCapture {
            rows: vec![row],
            cursor: Some(0),
            scalar: true,
            ..Default::default()
        }
    }
    fn batch_rows(&self) -> usize {
        self.batches.iter().map(|b| b.num_rows()).sum()
    }
}

impl UdfContext for BatchCapture {
    fn num_columns(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }
    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        let r = self
            .cursor
            .ok_or_else(|| UdfError::User("next() not called".into()))?;
        self.rows[r]
            .get(col)
            .ok_or_else(|| UdfError::Type(format!("column {col} out of range")))
    }
    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
        self.emitted_rows.push(values.to_vec());
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        if self.scalar {
            return Err(UdfError::User("next() in scalar context".into()));
        }
        let next = self.cursor.map_or(0, |c| c + 1);
        if next < self.rows.len() {
            self.cursor = Some(next);
            Ok(true)
        } else {
            self.cursor = None;
            Ok(false)
        }
    }
    fn emit_record_batch_ipc(&mut self, ipc: &[u8]) -> Result<(), UdfError> {
        let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(ipc), None)
            .map_err(|e| UdfError::Type(e.to_string()))?;
        for b in reader {
            self.batches
                .push(b.map_err(|e| UdfError::Type(e.to_string()))?);
        }
        Ok(())
    }
}

fn returns_ctx(row: Vec<Value>) -> TestContext {
    TestContext::scalar(row).with_emit_policy(EmitPolicy::Reject(UdfError::User(
        "RETURNS must not emit".into(),
    )))
}

fn set_returns_ctx(rows: Vec<Vec<Value>>) -> TestContext {
    TestContext::set(rows).with_emit_policy(EmitPolicy::Reject(UdfError::User(
        "RETURNS must not emit".into(),
    )))
}

fn params(n: i64, do_emit: i64) -> Vec<Value> {
    vec![Value::Int64(n), Value::Int64(do_emit)]
}

// ---------------------------------------------------------------------------
// Row generators and column classes
// ---------------------------------------------------------------------------

#[test]
fn native_row_is_int64_and_double() {
    let row = native_row(4);
    assert_eq!(row, [Value::Int64(4), Value::Double(6.0)]);
}

#[test]
fn strblock_row_has_typed_string_block_columns() {
    let row = strblock_row(42);
    assert!(matches!(row[0], Value::Int64(42)));
    assert!(matches!(&row[1], Value::Numeric(d) if d.scale == 2));
    assert!(matches!(row[2], Value::Date(_)));
    assert!(matches!(row[3], Value::Timestamp(_)));
}

#[test]
fn varchar_row_label_is_fifty_bytes_and_distinct_per_row() {
    let a = varchar_row(1);
    let b = varchar_row(2);
    match (&a[1], &b[1]) {
        (Value::String(x), Value::String(y)) => {
            assert_eq!(x.len(), LABEL_LEN);
            assert_eq!(y.len(), LABEL_LEN);
            assert_ne!(x, y);
        }
        other => panic!("expected strings, got {other:?}"),
    }
}

#[test]
fn date_cycles_every_ten_years() {
    assert_eq!(date(0), date(3650));
    assert_ne!(date(0), date(1));
}

#[test]
fn timestamp_advances_one_second_per_row_with_microsecond_fraction() {
    let t0 = timestamp(0);
    let t1 = timestamp(1);
    let delta = t1 - t0;
    assert!(delta >= chrono::Duration::seconds(1));
    assert!(delta < chrono::Duration::seconds(2));
    assert_eq!(timestamp(3).and_utc().timestamp_subsec_nanos() % 1000, 0);
}

#[test]
fn batches_match_row_generators() {
    let n = native_batch(10, 3).unwrap();
    assert_eq!(n.num_rows(), 3);
    let ks = n.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ks.value(2), 12);

    let s = strblock_batch(10, 3).unwrap();
    assert_eq!(s.num_columns(), 4);
    let amounts = s
        .column(1)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .unwrap();
    assert_eq!(amounts.value(1), amount(11).unscaled);
    let dates = s.column(2).as_any().downcast_ref::<Date32Array>().unwrap();
    assert_eq!(dates.value(0), date_to_epoch_days(date(10)));

    let v = varchar_batch(10, 3).unwrap();
    let labels = v.column(1).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(labels.value(2), label(12));
}

// ---------------------------------------------------------------------------
// SCALAR RETURNS
// ---------------------------------------------------------------------------

#[test]
fn sr_native_returns_k_plus_one() {
    let mut ctx = returns_ctx(native_row(41).to_vec());
    assert_eq!(sr_native(&mut ctx).unwrap(), Some(42));
}

#[test]
fn sr_strblock_doubles_amount_keeping_scale() {
    let mut ctx = returns_ctx(strblock_row(1).to_vec());
    let out = sr_strblock(&mut ctx).unwrap().unwrap();
    assert_eq!(out.unscaled, amount(1).unscaled * 2);
    assert_eq!(out.scale, 2);
}

#[test]
fn sr_varchar_returns_label_length() {
    let mut ctx = returns_ctx(varchar_row(7).to_vec());
    assert_eq!(sr_varchar(&mut ctx).unwrap(), Some(LABEL_LEN as i64));
}

#[test]
fn sr_native_null_k_returns_null() {
    let mut ctx = returns_ctx(vec![Value::Null, Value::Double(1.0)]);
    assert_eq!(sr_native(&mut ctx).unwrap(), None);
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

#[test]
fn gen_row_emits_n_rows_in_order() {
    let mut ctx = TestContext::scalar(params(5, 1));
    gen_strblock_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted().len(), 5);
    assert_eq!(ctx.emitted()[4], strblock_row(4).to_vec());
}

#[test]
fn gen_row_without_emit_yields_one_sentinel() {
    let mut ctx = TestContext::scalar(params(5, 0));
    gen_native_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted(), &[native_row(0).to_vec()]);
}

#[test]
fn gen_batch_emits_n_rows_across_chunks() {
    let n = CHUNK as i64 + 3;
    let mut ctx = BatchCapture::scalar(params(n, 1));
    gen_varchar_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batches.len(), 2);
    assert_eq!(ctx.batch_rows(), n as usize);
}

#[test]
fn gen_batch_without_emit_yields_one_row() {
    let mut ctx = BatchCapture::scalar(params(10, 0));
    gen_native_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batch_rows(), 1);
}

#[test]
fn setgen_reads_params_from_first_row_and_drains_group() {
    let mut ctx = TestContext::set(vec![params(3, 1), params(99, 1)]);
    setgen_native_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted().len(), 3);
}

#[test]
fn setgen_batch_emits_n_rows() {
    let mut ctx = BatchCapture::set(vec![params(7, 1)]);
    setgen_strblock_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batch_rows(), 7);
}

#[test]
fn setgen_on_empty_group_is_an_error() {
    let mut ctx = TestContext::set(vec![]);
    assert!(setgen_native_row(&mut ctx).is_err());
}

// ---------------------------------------------------------------------------
// Pass-through
// ---------------------------------------------------------------------------

#[test]
fn pt_native_emits_n_copies_of_k() {
    let mut ctx = TestContext::scalar(vec![Value::Int64(9), Value::Int64(4)]);
    pt_native(&mut ctx).unwrap();
    assert_eq!(ctx.emitted(), vec![vec![Value::Int64(9)]; 4].as_slice());
}

// ---------------------------------------------------------------------------
// SET RETURNS
// ---------------------------------------------------------------------------

#[test]
fn set_sum_native_sums_v() {
    let rows = (0..4).map(|i| native_row(i).to_vec()).collect();
    let mut ctx = set_returns_ctx(rows);
    assert_eq!(
        set_sum_native(&mut ctx).unwrap(),
        Some(0.0 + 1.5 + 3.0 + 4.5)
    );
}

#[test]
fn set_sum_strblock_sums_amount_at_scale_two() {
    let rows = (0..3).map(|i| strblock_row(i).to_vec()).collect();
    let mut ctx = set_returns_ctx(rows);
    let out = set_sum_strblock(&mut ctx).unwrap().unwrap();
    assert_eq!(
        out.unscaled,
        amount(0).unscaled + amount(1).unscaled + amount(2).unscaled
    );
    assert_eq!(out.scale, 2);
}

// ---------------------------------------------------------------------------
// SET EMITS re-emitters
// ---------------------------------------------------------------------------

#[test]
fn set_emit_row_reemits_every_input_row() {
    let rows: Vec<Vec<Value>> = (0..5).map(|i| varchar_row(i).to_vec()).collect();
    let mut ctx = TestContext::set(rows.clone());
    set_emit_varchar_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted(), rows.as_slice());
}

#[test]
fn set_emit_batch_reemits_every_input_row_typed() {
    let rows: Vec<Vec<Value>> = (0..5).map(|i| strblock_row(i).to_vec()).collect();
    let mut ctx = BatchCapture::set(rows);
    set_emit_strblock_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batch_rows(), 5);
    let b = &ctx.batches[0];
    let amounts = b
        .column(1)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .unwrap();
    assert_eq!(amounts.value(3), amount(3).unscaled);
    assert_eq!(*b.column(1).data_type(), DataType::Decimal128(18, 2));
}

#[test]
fn set_emit_batch_splits_at_chunk() {
    let rows: Vec<Vec<Value>> = (0..(CHUNK as i64 + 1))
        .map(|i| native_row(i).to_vec())
        .collect();
    let mut ctx = BatchCapture::set(rows);
    set_emit_native_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batches.len(), 2);
    assert_eq!(ctx.batches[0].num_rows(), CHUNK);
    assert_eq!(ctx.batches[1].num_rows(), 1);
}

#[test]
fn wide_row_has_24_columns_with_nulls_only_in_nullable_ones() {
    assert_eq!(WIDE_COLUMNS.len(), 24);
    for i in 0..50 {
        let row = wide_row(i);
        assert_eq!(row.len(), 24);
        for (col, v) in row.iter().enumerate() {
            let is_null = matches!(v, Value::Null);
            assert_eq!(is_null, wide_is_null(i, col), "row {i} col {col}");
            if is_null {
                assert!(WIDE_COLUMNS[col].2, "non-nullable column {col} is NULL");
            }
        }
    }
    // One in NULL_EVERY rows of a nullable column is NULL.
    let nulls = (0..1000).filter(|&i| wide_is_null(i, 1)).count();
    assert_eq!(nulls as i64, 1000 / NULL_EVERY);
    assert_eq!(WIDE_COLUMNS.iter().filter(|c| c.2).count(), 12);
}

#[test]
fn wide_text_varies_in_length_within_the_declared_maximum() {
    for (col, (_, ty, _)) in WIDE_COLUMNS.iter().enumerate().skip(16) {
        let max = bench_schema::varchar_size(ty).unwrap() as usize;
        let lens: std::collections::HashSet<usize> =
            (0..500).map(|i| wide_text(i, col).len()).collect();
        assert!(lens.iter().all(|&l| l >= 1 && l <= max), "col {col}");
        assert!(
            lens.len() > max / 2,
            "col {col}: only {} distinct lengths",
            lens.len()
        );
    }
    assert!(wide_ddl().starts_with("k DECIMAL(18,0), i1 DECIMAL(18,0)"));
    assert!(wide_ddl().ends_with("s200 VARCHAR(200)"));
}

#[test]
fn wide_batch_matches_wide_row_cell_for_cell() {
    let b = wide_batch(5, 20).unwrap();
    assert_eq!(b.num_columns(), 24);
    assert_eq!(b.num_rows(), 20);
    for (r, i) in (5..25).enumerate() {
        let row = wide_row(i);
        for (col, v) in row.iter().enumerate() {
            assert_eq!(
                b.column(col).is_null(r),
                matches!(v, Value::Null),
                "row {i} col {col}"
            );
        }
    }
    let ks = b.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ks.value(0), 5);
    let s200 = b.column(23).as_any().downcast_ref::<StringArray>().unwrap();
    let first_non_null = (5..25).find(|&i| !wide_is_null(i, 23)).unwrap();
    assert_eq!(
        s200.value((first_non_null - 5) as usize),
        wide_text(first_non_null, 23)
    );
}

#[test]
fn gen_wide_batch_honours_batch_rows() {
    let mut ctx = BatchCapture::scalar(vec![Value::Int64(25), Value::Int64(1), Value::Int64(10)]);
    gen_wide_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batches.len(), 3);
    assert_eq!(ctx.batch_rows(), 25);
    let mut ctx = BatchCapture::scalar(vec![Value::Int64(25), Value::Int64(1)]);
    gen_wide_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batches.len(), 1, "two parameters fall back to CHUNK");
}

#[test]
fn gen_wide_row_emits_n_rows() {
    let mut ctx = TestContext::scalar(params(7, 1));
    gen_wide_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted().len(), 7);
    assert_eq!(ctx.emitted()[3], wide_row(3));
}

#[test]
fn setgen_wide_batch_reads_three_params_from_first_row() {
    let mut ctx = BatchCapture::set(vec![
        vec![Value::Int64(12), Value::Int64(1), Value::Int64(5)],
        vec![Value::Int64(0), Value::Int64(0), Value::Int64(0)],
    ]);
    setgen_wide_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batches.len(), 3);
    assert_eq!(ctx.batch_rows(), 12);
}
