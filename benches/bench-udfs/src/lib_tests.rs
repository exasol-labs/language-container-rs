use super::*;
use arrow::array::{Array, Date32Array, Decimal128Array, Int64Array, StringArray};
use exasol_udf_sdk::test_support::{EmitPolicy, TestContext};

/// `TestContext` lacks `emit_record_batch_ipc`; this double captures batches.
#[derive(Default)]
struct BatchCapture {
    rows: Vec<Vec<Value>>,
    cursor: Option<usize>,
    scalar: bool,
    batches: Vec<RecordBatch>,
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
    fn emit(&mut self, _: &[Value]) -> Result<(), UdfError> {
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        if self.scalar {
            return Err(UdfError::User("next() in scalar context".into()));
        }
        let next = self.cursor.map_or(0, |c| c + 1);
        self.cursor = (next < self.rows.len()).then_some(next);
        Ok(self.cursor.is_some())
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

#[test]
fn row_values() {
    assert_eq!(native_row(4), [Value::Int64(4), Value::Double(6.0)]);
    let row = strblock_row(42);
    assert!(matches!(row[0], Value::Int64(42)));
    assert!(matches!(&row[1], Value::Numeric(d) if d.scale == 2));
    assert!(matches!(row[2], Value::Date(_)));
    assert!(matches!(row[3], Value::Timestamp(_)));
    let (Value::String(a), Value::String(b)) = (&varchar_row(1)[1], &varchar_row(2)[1]) else {
        panic!()
    };
    assert!(a.len() == LABEL_LEN && b.len() == LABEL_LEN && a != b);
    assert_eq!(date(0), date(3650));
    assert_ne!(date(0), date(1));
    let delta = timestamp(1) - timestamp(0);
    assert!(delta >= Duration::seconds(1) && delta < Duration::seconds(2));
    assert_eq!(timestamp(3).and_utc().timestamp_subsec_nanos() % 1000, 0);
}

#[test]
fn narrow_batches_match_row_generators() {
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
    assert_eq!(*s.column(1).data_type(), DataType::Decimal128(18, 2));
    let dates = s.column(2).as_any().downcast_ref::<Date32Array>().unwrap();
    assert_eq!(dates.value(0), epoch_days(date(10)));

    let v = varchar_batch(10, 3).unwrap();
    let labels = v.column(1).as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(labels.value(2), label(12));
}

#[test]
fn wide_rows_and_batches_agree_with_nulls_in_nullable_columns_only() {
    assert_eq!(WIDE_COLUMNS.len(), 24);
    assert_eq!(WIDE_COLUMNS.iter().filter(|c| c.2).count(), 12);
    assert_eq!(
        (0..1000).filter(|&i| wide_is_null(i, 1)).count() as i64,
        1000 / NULL_EVERY
    );
    assert!(wide_ddl().starts_with("k DECIMAL(18,0), i1 DECIMAL(18,0)"));

    let b = wide_batch(5, 20).unwrap();
    assert_eq!((b.num_columns(), b.num_rows()), (24, 20));
    for (r, i) in (5..25).enumerate() {
        for (col, v) in wide_row(i).iter().enumerate() {
            let is_null = matches!(v, Value::Null);
            assert_eq!(is_null, wide_is_null(i, col), "row {i} col {col}");
            assert!(!is_null || WIDE_COLUMNS[col].2);
            assert_eq!(b.column(col).is_null(r), is_null, "row {i} col {col}");
        }
    }
    assert_eq!(*b.column(10).data_type(), DataType::Decimal128(36, 10));
    let s200 = b.column(23).as_any().downcast_ref::<StringArray>().unwrap();
    let i = (5..25).find(|&i| !wide_is_null(i, 23)).unwrap();
    assert_eq!(s200.value((i - 5) as usize), wide_text(i, 23));

    for (col, (_, ty, _)) in WIDE_COLUMNS.iter().enumerate().skip(16) {
        let max = bench_schema::varchar_size(ty).unwrap() as usize;
        let lens: std::collections::HashSet<usize> =
            (0..500).map(|i| wide_text(i, col).len()).collect();
        assert!(lens.iter().all(|&l| (1..=max).contains(&l)), "col {col}");
        assert!(lens.len() > max / 2, "col {col}");
    }
}

#[test]
fn scalar_returns() {
    assert_eq!(
        sr_native(&mut returns_ctx(native_row(41).to_vec())).unwrap(),
        Some(42)
    );
    assert_eq!(
        sr_native(&mut returns_ctx(vec![Value::Null, Value::Double(1.0)])).unwrap(),
        None
    );
    let out = sr_strblock(&mut returns_ctx(strblock_row(1).to_vec()))
        .unwrap()
        .unwrap();
    assert_eq!((out.unscaled, out.scale), (amount(1).unscaled * 2, 2));
    assert_eq!(
        sr_varchar(&mut returns_ctx(varchar_row(7).to_vec())).unwrap(),
        Some(LABEL_LEN as i64)
    );
}

#[test]
fn row_generators_emit_n_rows_or_one_sentinel() {
    let mut ctx = TestContext::scalar(params(5, 1));
    gen_strblock_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted().len(), 5);
    assert_eq!(ctx.emitted()[4], strblock_row(4).to_vec());

    let mut ctx = TestContext::scalar(params(5, 0));
    gen_native_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted(), &[native_row(0).to_vec()]);

    let mut ctx = TestContext::scalar(params(7, 1));
    gen_wide_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted().len(), 7);
    assert_eq!(ctx.emitted()[3], wide_row(3));

    let mut ctx = TestContext::set(vec![params(3, 1), params(99, 1)]);
    setgen_native_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted().len(), 3);
    assert!(setgen_native_row(&mut TestContext::set(vec![])).is_err());
}

#[test]
fn batch_generators_chunk_by_default_or_by_batch_rows() {
    let n = CHUNK as i64 + 3;
    let mut ctx = BatchCapture::scalar(params(n, 1));
    gen_varchar_batch(&mut ctx).unwrap();
    assert_eq!((ctx.batches.len(), ctx.batch_rows()), (2, n as usize));

    let mut ctx = BatchCapture::scalar(params(10, 0));
    gen_native_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batch_rows(), 1);

    let mut ctx = BatchCapture::set(vec![params(7, 1)]);
    setgen_strblock_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batch_rows(), 7);

    let three = |n, e, b| vec![Value::Int64(n), Value::Int64(e), Value::Int64(b)];
    let mut ctx = BatchCapture::scalar(three(25, 1, 10));
    gen_wide_batch(&mut ctx).unwrap();
    assert_eq!((ctx.batches.len(), ctx.batch_rows()), (3, 25));
    let mut ctx = BatchCapture::scalar(params(25, 1));
    gen_wide_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batches.len(), 1);
    let mut ctx = BatchCapture::set(vec![three(12, 1, 5), three(0, 0, 0)]);
    setgen_wide_batch(&mut ctx).unwrap();
    assert_eq!((ctx.batches.len(), ctx.batch_rows()), (3, 12));
}

#[test]
fn pass_through_emits_n_copies_of_k() {
    let mut ctx = TestContext::scalar(vec![Value::Int64(9), Value::Int64(4)]);
    pt_native(&mut ctx).unwrap();
    assert_eq!(ctx.emitted(), vec![vec![Value::Int64(9)]; 4].as_slice());
}

#[test]
fn set_returns_sum_the_group() {
    let rows = (0..4).map(|i| native_row(i).to_vec()).collect();
    assert_eq!(
        set_sum_native(&mut set_returns_ctx(rows)).unwrap(),
        Some(0.0 + 1.5 + 3.0 + 4.5)
    );
    let rows = (0..3).map(|i| strblock_row(i).to_vec()).collect();
    let out = set_sum_strblock(&mut set_returns_ctx(rows))
        .unwrap()
        .unwrap();
    assert_eq!(
        out.unscaled,
        (0..3).map(|i| amount(i).unscaled).sum::<i128>()
    );
    assert_eq!(out.scale, 2);
}

#[test]
fn set_emits_reemit_every_input_row() {
    let rows: Vec<Vec<Value>> = (0..5).map(|i| varchar_row(i).to_vec()).collect();
    let mut ctx = TestContext::set(rows.clone());
    set_emit_varchar_row(&mut ctx).unwrap();
    assert_eq!(ctx.emitted(), rows.as_slice());

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

    let rows: Vec<Vec<Value>> = (0..(CHUNK as i64 + 1))
        .map(|i| native_row(i).to_vec())
        .collect();
    let mut ctx = BatchCapture::set(rows);
    set_emit_native_batch(&mut ctx).unwrap();
    assert_eq!(ctx.batches.len(), 2);
    assert_eq!(ctx.batches[0].num_rows(), CHUNK);
    assert_eq!(ctx.batches[1].num_rows(), 1);
}
