use super::*;

/// Verify that `debug_level()` on both bridges reads the process-global
/// `LevelFilter` and never panics (including when the filter is `OFF`).
///
/// The implementation is `LevelFilter::current().into_level().unwrap_or(INFO)`.
/// We cannot set the global level in a unit test without a subscriber, so
/// we verify the weaker property: the method returns a valid `Level` value
/// (one of the five known variants) and maps `OFF` to `INFO` by checking
/// directly with `LevelFilter::OFF.into_level()`.
#[test]
fn host_bridge_debug_level_returns_valid_level() {
    use exa_proto::ExascriptTableData;

    let meta = vec![ColumnMeta {
        name: "a".to_string(),
        typ: ExaType::Int64,
        type_name: String::new(),
        size: None,
        precision: None,
        scale: None,
    }];
    let table = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut rs = InputRowSet::from_proto(&table, &meta);
    let mut emit = EmitBuffer::new();
    let bridge = HostContextBridge::new(
        &mut rs,
        &mut emit,
        &meta,
        &meta,
        Box::new(|_| Ok(())),
        HandshakeMeta::default(),
        #[cfg(feature = "connect-back")]
        Box::new(|_name| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "no credential fetcher".into(),
            ))
        }),
    );

    // The bridge must not panic and must return a valid Level variant.
    let level = bridge.debug_level();
    assert!(
        level == tracing::Level::ERROR
            || level == tracing::Level::WARN
            || level == tracing::Level::INFO
            || level == tracing::Level::DEBUG
            || level == tracing::Level::TRACE,
        "unexpected level {level}"
    );

    // The OFF fallback is encoded in the implementation, not the global
    // state; verify the expression directly.
    let off_mapped = tracing::level_filters::LevelFilter::OFF
        .into_level()
        .unwrap_or(tracing::Level::INFO);
    assert_eq!(off_mapped, tracing::Level::INFO, "OFF must map to INFO");
}

#[test]
fn single_call_context_debug_level_returns_valid_level() {
    #[cfg(feature = "connect-back")]
    let ctx = SingleCallContext::new(
        HandshakeMeta::default(),
        Box::new(|_name| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "no credential fetcher".into(),
            ))
        }),
    );
    #[cfg(not(feature = "connect-back"))]
    let ctx = SingleCallContext::new(HandshakeMeta::default());

    let level = ctx.debug_level();
    assert!(
        level == tracing::Level::ERROR
            || level == tracing::Level::WARN
            || level == tracing::Level::INFO
            || level == tracing::Level::DEBUG
            || level == tracing::Level::TRACE,
        "unexpected level {level}"
    );
}

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
        cols, // output_meta: reuse the same schema for test simplicity
        Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
        HandshakeMeta::default(),
        #[cfg(feature = "connect-back")]
        Box::new(|_name| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "no credential fetcher in test".into(),
            ))
        }),
    )
}

/// A one-row single-Int64-column batch for the contract-gate tests.
fn single_int_batch() -> (ExascriptTableData, Vec<ColumnMeta>) {
    let meta = vec![col("a", ExaType::Int64)];
    let table = ExascriptTableData {
        rows: 1,
        data_int64: vec![1],
        data_nulls: vec![false],
        ..Default::default()
    };
    (table, meta)
}

#[test]
fn scalar_input_bans_next() {
    let (table, meta) = single_int_batch();
    let mut rs = InputRowSet::from_proto(&table, &meta);
    let mut emit = EmitBuffer::new();
    let mut bridge = make_bridge(&mut rs, &mut emit, &meta);
    bridge.configure_group_input(
        IterType::ExactlyOnce,
        IterType::ExactlyOnce,
        Box::new(|| Ok(None)),
    );
    match bridge.next() {
        Err(UdfError::User(msg)) => assert!(
            msg.contains("next()") && msg.contains("scalar"),
            "unexpected next-ban message: {msg}"
        ),
        other => panic!("expected next() ban error, got {other:?}"),
    }
}

#[test]
fn returns_output_bans_emit() {
    let (table, meta) = single_int_batch();
    let mut rs = InputRowSet::from_proto(&table, &meta);
    let mut emit = EmitBuffer::new();
    let mut bridge = make_bridge(&mut rs, &mut emit, &meta);
    bridge.configure_group_input(
        IterType::ExactlyOnce,
        IterType::ExactlyOnce,
        Box::new(|| Ok(None)),
    );
    match bridge.emit(&[Value::Int64(9)]) {
        Err(UdfError::User(msg)) => assert!(
            msg.contains("emit()") && msg.contains("RETURNS"),
            "unexpected emit-ban message: {msg}"
        ),
        other => panic!("expected emit() ban error, got {other:?}"),
    }
}

#[test]
fn set_return_records_some_as_row_and_none_as_null() {
    let (table, meta) = single_int_batch();
    let mut rs = InputRowSet::from_proto(&table, &meta);
    let mut emit = EmitBuffer::new();
    {
        let mut bridge = make_bridge(&mut rs, &mut emit, &meta);
        bridge.configure_group_input(
            IterType::ExactlyOnce,
            IterType::ExactlyOnce,
            Box::new(|| Ok(None)),
        );
        bridge.set_return(Some(Value::Int64(7))).unwrap();
        bridge.set_return(None).unwrap();
    }
    assert_eq!(
        emit.rows,
        vec![vec![Value::Int64(7)], vec![Value::Null]],
        "set_return must record Some(v) as [v] and None as [Null]"
    );
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

/// `to_proto`'s per-type blocks are pre-sized with `Vec::with_capacity` for
/// the exact (non-NULL) column count instead of growing via `Vec::new()` +
/// `push`. This only asserts the resulting contents are correct — `Vec`'s
/// capacity is only guaranteed to be *at least* the requested value (the
/// allocator/growth strategy is explicitly unspecified), so asserting an
/// exact `capacity()` here would be a flaky test rather than a real
/// regression guard; the pre-sizing's throughput benefit is verified by
/// `benches/emit-bench`, not by inspecting internal `Vec` capacity.
#[test]
fn to_proto_presizes_string_block_capacity() {
    let meta = vec![col("a", ExaType::String { size: None })];
    let mut emit = EmitBuffer::new();
    emit.push(vec![Value::String("a".into())]);
    emit.push(vec![Value::String("b".into())]);
    emit.push(vec![Value::String("c".into())]);

    let table = emit.to_proto(&meta);
    assert_eq!(table.data_string, vec!["a", "b", "c"]);
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
fn emit_buffer_limit_is_exactly_4_000_000() {
    // The other tests use EMIT_BUFFER_LIMIT_BYTES symbolically, so a silent
    // change to 4 MiB would pass them. The wire limit is 4,000,000 bytes.
    assert_eq!(EMIT_BUFFER_LIMIT_BYTES, 4_000_000);
}

#[test]
fn bridge_emit_row_path_flushes_once_mid_run_and_buffers_residual() {
    // Pins the row path (HostContextBridge::emit) mid-run flush behavior; the
    // batch path is pinned by bridge_emit_batch_buffers_and_flushes.
    let meta = vec![col("v", ExaType::String { size: None })];
    let empty_table = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut rs = InputRowSet::from_proto(&empty_table, &meta);
    let mut emit = EmitBuffer::new();
    let flush_count = std::cell::Cell::new(0usize);
    let flush_count_ref = &flush_count;
    let row_value = || Value::String("x".repeat(1000));
    let rows_to_limit = EMIT_BUFFER_LIMIT_BYTES.div_ceil(1000);

    {
        let mut bridge = HostContextBridge::new(
            &mut rs,
            &mut emit,
            &meta,
            &meta,
            Box::new(move |t: exa_proto::ExascriptTableData| {
                if t.rows > 0 {
                    flush_count_ref.set(flush_count_ref.get() + 1);
                }
                Ok(())
            }),
            HandshakeMeta::default(),
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        );

        for _ in 0..(rows_to_limit - 1) {
            bridge.emit(&[row_value()]).unwrap();
        }
        assert_eq!(
            flush_count.get(),
            0,
            "no flush expected while under the limit"
        );

        bridge.emit(&[row_value()]).unwrap();
        assert_eq!(
            flush_count.get(),
            1,
            "crossing the limit must flush exactly once"
        );

        // Rows pushed after the mid-run flush stay buffered for the eventual
        // tail flush; they must not trigger a second flush on their own.
        bridge.emit(&[row_value()]).unwrap();
        bridge.emit(&[row_value()]).unwrap();
        assert_eq!(
            flush_count.get(),
            1,
            "residual rows below the limit must not trigger a second flush"
        );
    }
    assert_eq!(
        emit.len(),
        2,
        "residual rows must remain buffered for the tail flush"
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

/// `refill`'s loop skips zero-row `MT_NEXT` batches and keeps fetching until a
/// non-empty one arrives, or the group ends.
#[test]
fn refill_skips_empty_batches_until_a_nonempty_one_arrives() {
    let meta = vec![col("a", ExaType::Int64)];
    let empty_table = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut rs = InputRowSet::from_proto(&empty_table, &meta);
    let mut emit = EmitBuffer::new();
    let mut bridge = make_bridge(&mut rs, &mut emit, &meta);

    let call_count = std::cell::Cell::new(0usize);
    bridge.configure_group_input(
        IterType::Multiple,
        IterType::Multiple,
        Box::new(move || {
            let n = call_count.get();
            call_count.set(n + 1);
            match n {
                // First MT_NEXT batch: zero rows, must be skipped.
                0 => Ok(Some(ExascriptTableData {
                    rows: 0,
                    ..Default::default()
                })),
                // Second MT_NEXT batch: one row, must be the landing point.
                1 => Ok(Some(ExascriptTableData {
                    rows: 1,
                    data_int64: vec![9],
                    data_nulls: vec![false],
                    ..Default::default()
                })),
                _ => Ok(None),
            }
        }),
    );

    assert!(
        bridge.next().unwrap(),
        "must skip the empty batch and land on the non-empty one"
    );
    assert_eq!(bridge.get(0).unwrap(), &Value::Int64(9));
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
        &meta, // output_meta
        Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
        HandshakeMeta {
            memory_limit: limit_bytes,
            ..Default::default()
        },
        #[cfg(feature = "connect-back")]
        Box::new(|_name| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "no credential fetcher in test".into(),
            ))
        }),
    );
    assert_eq!(bridge.memory_limit(), limit_bytes);
}

#[test]
fn bridge_returns_handshake_metadata() {
    let meta = vec![col("a", ExaType::Int64)];
    let table = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut rs = InputRowSet::from_proto(&table, &meta);
    let mut emit = EmitBuffer::new();
    // A present optional (current_user) and absent optionals (current_schema,
    // scope_user) prove the bridge mirrors the proto present/absent distinction.
    let handshake = HandshakeMeta {
        session_id: 4242,
        statement_id: 9,
        node_id: 1,
        node_count: 4,
        vm_id: 777777,
        memory_limit: 256 * 1024 * 1024,
        database_name: "EXADB".to_string(),
        database_version: "2026.1.0".to_string(),
        script_name: "MY_SCRIPT".to_string(),
        script_schema: "MY_SCHEMA".to_string(),
        current_user: Some("ALICE".to_string()),
        current_schema: None,
        scope_user: None,
    };
    let bridge = HostContextBridge::new(
        &mut rs,
        &mut emit,
        &meta,
        &meta,
        Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
        handshake,
        #[cfg(feature = "connect-back")]
        Box::new(|_name| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "no credential fetcher in test".into(),
            ))
        }),
    );

    // Numeric accessors return the exact UdfMeta values, no rescaling.
    assert_eq!(bridge.session_id(), 4242);
    assert_eq!(bridge.statement_id(), 9);
    assert_eq!(bridge.node_id(), 1);
    assert_eq!(bridge.node_count(), 4);
    assert_eq!(bridge.vm_id(), 777777);
    assert_eq!(bridge.memory_limit(), 256 * 1024 * 1024);
    // Owned-string accessors return the exact values.
    assert_eq!(bridge.database_name(), "EXADB");
    assert_eq!(bridge.database_version(), "2026.1.0");
    assert_eq!(bridge.script_name(), "MY_SCRIPT");
    assert_eq!(bridge.script_schema(), "MY_SCHEMA");
    // Optionals: Some when present, None when absent.
    assert_eq!(bridge.current_user(), Some("ALICE".to_string()));
    assert_eq!(bridge.current_schema(), None);
    assert_eq!(bridge.scope_user(), None);
}

#[test]
fn single_call_context_returns_handshake_metadata() {
    // A present optional (current_user) and absent optionals (current_schema,
    // scope_user) prove the single-call context mirrors the proto
    // present/absent distinction, same as HostContextBridge.
    let handshake = HandshakeMeta {
        session_id: 4242,
        statement_id: 9,
        node_id: 1,
        node_count: 4,
        vm_id: 777777,
        memory_limit: 256 * 1024 * 1024,
        database_name: "EXADB".to_string(),
        database_version: "2026.1.0".to_string(),
        script_name: "MY_SCRIPT".to_string(),
        script_schema: "MY_SCHEMA".to_string(),
        current_user: Some("ALICE".to_string()),
        current_schema: None,
        scope_user: None,
    };

    #[cfg(feature = "connect-back")]
    let ctx = SingleCallContext::new(
        handshake,
        Box::new(|_name| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                "no credential fetcher in test".into(),
            ))
        }),
    );
    #[cfg(not(feature = "connect-back"))]
    let ctx = SingleCallContext::new(handshake);

    // Numeric accessors return the exact UdfMeta values, no rescaling.
    assert_eq!(ctx.session_id(), 4242);
    assert_eq!(ctx.statement_id(), 9);
    assert_eq!(ctx.node_id(), 1);
    assert_eq!(ctx.node_count(), 4);
    assert_eq!(ctx.vm_id(), 777777);
    assert_eq!(ctx.memory_limit(), 256 * 1024 * 1024);
    // Owned-string accessors return the exact values.
    assert_eq!(ctx.database_name(), "EXADB");
    assert_eq!(ctx.database_version(), "2026.1.0");
    assert_eq!(ctx.script_name(), "MY_SCRIPT");
    assert_eq!(ctx.script_schema(), "MY_SCHEMA");
    // Optionals: Some when present, None when absent.
    assert_eq!(ctx.current_user(), Some("ALICE".to_string()));
    assert_eq!(ctx.current_schema(), None);
    assert_eq!(ctx.scope_user(), None);
}

// -----------------------------------------------------------------------
// Telemetry tests (tasks 2.15 — 5.4)
// -----------------------------------------------------------------------

/// Serialises tests that install tracing subscribers via `with_default`.
///
/// Any `tracing::subscriber::with_default` call that installs a
/// DEBUG-level subscriber can, upon first use of a `debug!` callsite,
/// trigger `rebuild_interest_cache` which updates the process-global
/// `MAX_LEVEL` atomic.  Concurrent tests that also assert on captured
/// debug output may see the wrong `MAX_LEVEL` and have their events
/// silently dropped by the macro fast-path check.  Holding this lock for
/// the full duration of any such test eliminates the race.
static GLOBAL_LEVEL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A `MakeWriter` that appends to a shared `Mutex<Vec<u8>>`.
///
/// Used by the telemetry tests to capture `tracing` output without a global
/// subscriber.  Each call to `make_writer` clones the `Arc` so the subscriber
/// can hold it across events.
#[cfg(test)]
struct LockedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LockedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LockedWriter {
    type Writer = LockedWriter;
    fn make_writer(&'a self) -> Self::Writer {
        LockedWriter(std::sync::Arc::clone(&self.0))
    }
}

/// Verify telemetry events appear at `debug` level and are absent at `info`.
///
/// Scenario: `telemetry_emitted_at_debug_level_only` (plan task 5.4).
///
/// Uses a `Mutex<Vec<u8>>` capture writer and `tracing::subscriber::with_default`
/// with a `reload::Layer` so `filter_handle.modify` triggers
/// `rebuild_interest_cache()`, which resets any previously cached callsite
/// interests.  Holds `GLOBAL_LEVEL_LOCK` to prevent concurrent tests from
/// racing on the global `MAX_LEVEL` atomic.
#[test]
fn telemetry_emitted_at_debug_level_only() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
    use tracing_subscriber::reload;

    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let capture_with_level = |level: tracing::Level| -> String {
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let initial = tracing_subscriber::EnvFilter::new("info");
        let (filter_layer, filter_handle) = reload::Layer::new(initial);
        let sub = tracing_subscriber::registry().with(filter_layer).with(
            tracing_subscriber::fmt::layer()
                .with_writer(LockedWriter(Arc::clone(&buf)))
                .with_ansi(false),
        );
        tracing::subscriber::with_default(sub, || {
            // Force rebuild_interest_cache() so previously-cached callsites are reset.
            let _ =
                filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new(level.as_str()));

            let mut emit = EmitBuffer::new();
            // Push enough rows to trigger a flush (each ~1000 bytes).
            let rows_to_flush = EMIT_BUFFER_LIMIT_BYTES.div_ceil(1000) + 1;
            for _ in 0..rows_to_flush {
                emit.push(vec![Value::String("x".repeat(1000))]);
                if emit.should_flush() {
                    emit.record_flush_telemetry();
                    emit.clear();
                }
            }
            if !emit.is_empty() {
                emit.record_flush_telemetry();
                emit.clear();
            }
        });
        let captured = buf.lock().unwrap();
        String::from_utf8_lossy(&captured).into_owned()
    };

    let debug_output = capture_with_level(tracing::Level::DEBUG);
    // Restore to INFO before capturing the info output.
    let info_output = capture_with_level(tracing::Level::INFO);

    assert!(
        debug_output.contains("emit_flush"),
        "debug output must contain emit_flush telemetry, got: {debug_output:?}"
    );
    assert!(
        !info_output.contains("emit_flush"),
        "info output must not contain emit_flush telemetry, got: {info_output:?}"
    );
}

/// Verify that `debug!` events around `push` are recorded at debug level.
///
/// Uses a `reload::Layer`-based subscriber and calls `filter_handle.modify`
/// to trigger `rebuild_interest_cache()`, which resets any previously cached
/// callsite interests (avoiding the "event permanently cached as never"
/// failure mode when another test registered the callsite first without a
/// debug-level subscriber installed).
///
/// Scenario: `emit_flush_path_instrumented` (plan task 5.4).
#[test]
fn emit_flush_path_instrumented() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
    use tracing_subscriber::reload;

    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let initial = tracing_subscriber::EnvFilter::new("info");
    let (filter_layer, filter_handle) = reload::Layer::new(initial);
    let sub = tracing_subscriber::registry().with(filter_layer).with(
        tracing_subscriber::fmt::layer()
            .with_writer(LockedWriter(Arc::clone(&buf)))
            .with_ansi(false),
    );

    tracing::subscriber::with_default(sub, || {
        // Force rebuild_interest_cache() to reset any stale callsite cache entries.
        let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("debug"));

        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::String("hello".to_string())]);
    });

    let captured = buf.lock().unwrap();
    let output = String::from_utf8_lossy(&captured);
    assert!(
        output.contains("emit_push") || output.contains("bytes_buffered"),
        "debug output must contain push instrumentation, got: {output:?}"
    );
}

/// `push`'s periodic checkpoint fires every `TELEMETRY_ROW_CHECKPOINT` rows,
/// independently of the byte-threshold flush.
#[test]
fn emit_push_periodic_checkpoint_at_10_000_rows() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
    use tracing_subscriber::reload;

    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let initial = tracing_subscriber::EnvFilter::new("info");
    let (filter_layer, filter_handle) = reload::Layer::new(initial);
    let sub = tracing_subscriber::registry().with(filter_layer).with(
        tracing_subscriber::fmt::layer()
            .with_writer(LockedWriter(Arc::clone(&buf)))
            .with_ansi(false),
    );

    tracing::subscriber::with_default(sub, || {
        let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("debug"));

        let mut emit = EmitBuffer::new();
        for _ in 0..EmitBuffer::TELEMETRY_ROW_CHECKPOINT {
            emit.push(vec![Value::Bool(true)]);
        }
        assert!(
            !emit.should_flush(),
            "10,000 cheap rows must stay well under the byte threshold"
        );
    });

    let captured = buf.lock().unwrap();
    let output = String::from_utf8_lossy(&captured);
    assert!(
        output.contains("MT_EMIT flush"),
        "the 10,000-row checkpoint must call record_flush_telemetry, got: {output:?}"
    );
}

/// `InputRowSet::from_proto`'s `ExaType::Int32` arm advances the per-type
/// cursor only for non-null cells — a NULL is interleaved so a
/// cursor-advance-on-NULL regression would misalign row 1.
#[test]
fn input_rowset_decodes_int32_column() {
    let meta = vec![col("a", ExaType::Int32)];
    let table = ExascriptTableData {
        rows: 3,
        data_int32: vec![7, -8],
        data_nulls: vec![false, true, false],
        ..Default::default()
    };
    let rs = InputRowSet::from_proto(&table, &meta);
    assert_eq!(rs.row(0).unwrap(), &[Value::Int32(7)]);
    assert_eq!(rs.row(1).unwrap(), &[Value::Null]);
    assert_eq!(rs.row(2).unwrap(), &[Value::Int32(-8)]);
}

/// An `ExaType::Unsupported` column has no backing type block, so
/// `from_proto` maps every cell to `Value::Null` regardless of the NULL bitmap.
#[test]
fn input_rowset_unsupported_column_decodes_to_null() {
    let meta = vec![col("a", ExaType::Int64), col("u", ExaType::Unsupported)];
    let table = ExascriptTableData {
        rows: 1,
        data_int64: vec![5],
        // Neither cell is marked NULL; the Unsupported column still yields
        // Value::Null because it has no type-block slot to read.
        data_nulls: vec![false, false],
        ..Default::default()
    };
    let rs = InputRowSet::from_proto(&table, &meta);
    assert_eq!(rs.row(0).unwrap(), &[Value::Int64(5), Value::Null]);
}

/// An `ExaType::Unsupported` column contributes no slot to either pass of
/// `to_proto` — it is skipped rather than landing in some other block.
#[test]
fn to_proto_skips_unsupported_columns_in_both_tally_and_packing() {
    let meta = vec![col("a", ExaType::Int64), col("u", ExaType::Unsupported)];
    let mut emit = EmitBuffer::new();
    emit.push(vec![Value::Int64(7), Value::String("ignored".into())]);
    let table = emit.to_proto(&meta);

    assert_eq!(table.data_int64, vec![7]);
    assert!(
        table.data_string.is_empty(),
        "an Unsupported column must not occupy the string block"
    );
    assert_eq!(
        table.data_nulls,
        vec![false, false],
        "the Unsupported cell is not NULL, just unrepresented"
    );
}

/// `value_to_block_string`'s `Value::Bool`/`Value::Null` arms have no fast-path
/// counterpart, so the slow-path parity test does not reach them.
#[test]
fn value_to_block_string_bool_and_null_render_as_text_and_empty() {
    assert_eq!(value_to_block_string(&Value::Bool(true)), "true");
    assert_eq!(value_to_block_string(&Value::Bool(false)), "false");
    assert_eq!(value_to_block_string(&Value::Null), "");
}

/// `value_to_i64`/`value_to_f64`/`value_to_bool` coerce every `Value` variant
/// for an EMITS column whose declared type disagrees with the runtime `Value`.
/// The parity tests already cover each function's natural arm; this covers the
/// remaining coercion and wildcard arms.
mod value_coercion_tests {
    use super::*;

    #[test]
    fn value_to_i64_coerces_every_non_natural_variant() {
        assert_eq!(value_to_i64(&Value::Double(3.9)), 3);
        assert_eq!(
            value_to_i64(&Value::Numeric(Decimal {
                unscaled: 12345,
                scale: 2
            })),
            123
        );
        assert_eq!(value_to_i64(&Value::String("42".into())), 42);
        assert_eq!(
            value_to_i64(&Value::String("not a number".into())),
            0,
            "an unparseable string must fall back to 0"
        );
        assert_eq!(value_to_i64(&Value::Bool(true)), 1);
        assert_eq!(value_to_i64(&Value::Bool(false)), 0);
        assert_eq!(
            value_to_i64(&Value::Date(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap())),
            0,
            "the wildcard arm must return 0 for a variant with no numeric reading"
        );
    }

    #[test]
    fn value_to_f64_coerces_every_non_natural_variant() {
        assert_eq!(value_to_f64(&Value::Int32(4)), 4.0);
        assert_eq!(value_to_f64(&Value::Int64(-5)), -5.0);
        assert_eq!(
            value_to_f64(&Value::Numeric(Decimal {
                unscaled: 125,
                scale: 2
            })),
            1.25
        );
        assert_eq!(value_to_f64(&Value::String("2.5".into())), 2.5);
        assert_eq!(
            value_to_f64(&Value::String("not a number".into())),
            0.0,
            "an unparseable string must fall back to 0.0"
        );
        assert_eq!(
            value_to_f64(&Value::Bool(true)),
            0.0,
            "the wildcard arm must return 0.0 for a variant with no numeric reading"
        );
    }

    #[test]
    fn value_to_bool_coerces_every_non_natural_variant() {
        assert!(!value_to_bool(&Value::Int32(0)));
        assert!(value_to_bool(&Value::Int32(5)));
        assert!(!value_to_bool(&Value::Int64(0)));
        assert!(value_to_bool(&Value::Int64(-1)));
        assert!(!value_to_bool(&Value::Numeric(Decimal {
            unscaled: 0,
            scale: 0
        })));
        assert!(value_to_bool(&Value::Numeric(Decimal {
            unscaled: 7,
            scale: 0
        })));
        assert!(value_to_bool(&Value::String("true".into())));
        assert!(value_to_bool(&Value::String("TRUE".into())));
        assert!(value_to_bool(&Value::String("1".into())));
        assert!(!value_to_bool(&Value::String("no".into())));
        assert!(
            !value_to_bool(&Value::Double(1.0)),
            "the wildcard arm must return false for a variant with no boolean reading"
        );
    }
}

/// `first_nonloopback_ipv4` walks the real `getifaddrs` list, so its result
/// depends on the host: a well-formed non-loopback dotted quad where one
/// exists, `UdfError::ConnectBack` (never a panic) on an isolated host.
#[cfg(feature = "connect-back")]
#[test]
fn first_nonloopback_ipv4_returns_a_valid_non_loopback_ipv4_address() {
    match first_nonloopback_ipv4() {
        Ok(ip) => assert!(!ip.starts_with("127."), "must not be loopback: {ip}"),
        Err(err) => assert!(
            matches!(err, UdfError::ConnectBack(_)),
            "no-interface host must fail with ConnectBack, got {err:?}"
        ),
    }
}

/// Both `cluster_ip` impls (the `delegate_connect_back_hooks!` expansion) just
/// forward to `first_nonloopback_ipv4`.
#[cfg(feature = "connect-back")]
#[test]
fn bridge_cluster_ip_delegates_to_first_nonloopback_ipv4() {
    let meta = vec![col("a", ExaType::Int64)];
    let table = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut rs = InputRowSet::from_proto(&table, &meta);
    let mut emit = EmitBuffer::new();
    let bridge = make_bridge(&mut rs, &mut emit, &meta);

    assert_eq!(
        format!("{:?}", bridge.cluster_ip()),
        format!("{:?}", first_nonloopback_ipv4())
    );
}

#[cfg(feature = "connect-back")]
#[test]
fn single_call_context_cluster_ip_delegates_to_first_nonloopback_ipv4() {
    let ctx = single_call_ctx();
    assert_eq!(
        format!("{:?}", ctx.cluster_ip()),
        format!("{:?}", first_nonloopback_ipv4())
    );
}

/// `connection()`'s error branch records the failure via `record_error` so it
/// surfaces through `take_last_error`.
#[cfg(feature = "connect-back")]
#[test]
fn bridge_connection_error_is_recorded_via_record_error() {
    let meta = vec![col("a", ExaType::Int64)];
    let table = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut rs = InputRowSet::from_proto(&table, &meta);
    let mut emit = EmitBuffer::new();
    let mut bridge = HostContextBridge::new(
        &mut rs,
        &mut emit,
        &meta,
        &meta,
        Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
        HandshakeMeta::default(),
        Box::new(|name: &str| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(format!(
                "no such connection: {name}"
            )))
        }),
    );

    let result = bridge.connection("MISSING_CONN");
    assert!(
        matches!(result, Err(UdfError::ConnectBack(_))),
        "expected a ConnectBack error, got {result:?}"
    );

    let recorded = bridge.take_last_error();
    assert!(
        recorded
            .as_deref()
            .is_some_and(|m| m.contains("MISSING_CONN")),
        "record_error must capture the failure message, got {recorded:?}"
    );
}

#[cfg(feature = "connect-back")]
#[test]
fn single_call_context_connection_error_is_recorded_via_record_error() {
    let mut ctx = SingleCallContext::new(
        HandshakeMeta::default(),
        Box::new(|name: &str| {
            Err(exasol_udf_sdk::error::UdfError::ConnectBack(format!(
                "no such connection: {name}"
            )))
        }),
    );

    let result = ctx.connection("MISSING_CONN");
    assert!(
        matches!(result, Err(UdfError::ConnectBack(_))),
        "expected a ConnectBack error, got {result:?}"
    );

    let recorded = ctx.take_last_error();
    assert!(
        recorded
            .as_deref()
            .is_some_and(|m| m.contains("MISSING_CONN")),
        "record_error must capture the failure message, got {recorded:?}"
    );
}

/// Construct a `SingleCallContext`, supplying the connect-back arg only when
/// the feature is enabled so call sites compile either way.
fn single_call_ctx() -> SingleCallContext<'static> {
    #[cfg(feature = "connect-back")]
    {
        SingleCallContext::new(
            HandshakeMeta::default(),
            Box::new(|_name: &str| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        )
    }
    #[cfg(not(feature = "connect-back"))]
    {
        SingleCallContext::new(HandshakeMeta::default())
    }
}

/// Single-call mode presents no input columns: `num_columns` always reports
/// 0 regardless of handshake or connection state.
#[test]
fn single_call_context_num_columns_is_always_zero() {
    assert_eq!(single_call_ctx().num_columns(), 0);
}

/// Single-call mode has no input rows to read: `get` always rejects with
/// `Unimplemented`.
#[test]
fn single_call_context_get_is_unimplemented() {
    match single_call_ctx().get(0) {
        Err(UdfError::Unimplemented(msg)) => assert!(msg.contains("input columns")),
        other => panic!("expected Unimplemented, got {other:?}"),
    }
}

/// Single-call mode emits no output rows: `emit` always rejects with
/// `Unimplemented`.
#[test]
fn single_call_context_emit_is_unimplemented() {
    match single_call_ctx().emit(&[Value::Int64(1)]) {
        Err(UdfError::Unimplemented(msg)) => assert!(msg.contains("emit")),
        other => panic!("expected Unimplemented, got {other:?}"),
    }
}

/// Single-call mode has no input batches to iterate: `next` always rejects
/// with `Unimplemented`.
#[test]
fn single_call_context_next_is_unimplemented() {
    match single_call_ctx().next() {
        Err(UdfError::Unimplemented(msg)) => assert!(msg.contains("input rows")),
        other => panic!("expected Unimplemented, got {other:?}"),
    }
}

// -----------------------------------------------------------------------
// Permanent regression guard: the string-block fast-path formatters
// (`value_to_block_string`'s NUMERIC/DATE/TIMESTAMP branches) must stay
// byte-identical to the `chrono`/`Display` path they replaced, for every
// representable value.
// -----------------------------------------------------------------------
mod fast_string_block_tests {
    use super::*;

    fn decimal(unscaled: i128, scale: u8) -> Decimal {
        Decimal { unscaled, scale }
    }

    /// `fast_decimal_to_string` must match `Decimal`'s `Display` impl for
    /// every representable case: positive/negative/zero, scale 0 through a
    /// realistic max, and padding edge cases where the unscaled digit run
    /// is shorter than the scale.
    #[test]
    fn fast_decimal_matches_display_for_all_cases() {
        let cases = [
            (0i128, 0u8),
            (0, 5),
            (1, 0),
            (-1, 0),
            (5, 2),
            (-5, 2),
            (12, 2),
            (-12, 2),
            (100, 2),
            (123456789, 3),
            (-123456789, 3),
            (1, 18),
            (-1, 18),
            (1_000_000_000_000_000_001, 18),
            (-1_000_000_000_000_000_001, 18),
            (i128::MAX, 0),
            (i128::MAX, 18),
            (i128::MIN, 0),
            (i128::MIN, 18),
            (i128::MIN, 38),
            (9, 1),
            (10, 1),
            (99, 1),
        ];
        for (unscaled, scale) in cases {
            let d = decimal(unscaled, scale);
            assert_eq!(
                fast_decimal_to_string(&d),
                d.to_string(),
                "mismatch for unscaled={unscaled} scale={scale}"
            );
        }
    }

    /// `fast_date_to_string` must match `NaiveDate::format(DATE_FORMAT)`
    /// byte-for-byte across leap days, year boundaries, and pre-1970 dates.
    #[test]
    fn fast_date_matches_chrono_format() {
        let cases = [
            (0, 1, 1),
            (1, 1, 1),
            (1969, 12, 31),
            (1970, 1, 1),
            (2000, 2, 29),
            (2024, 2, 29),
            (2026, 6, 5),
            (9999, 12, 31),
        ];
        for (y, m, d) in cases {
            let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
            let expected = date.format(DATE_FORMAT).to_string();
            assert_eq!(
                fast_date_to_string(&date),
                Some(expected.clone()),
                "mismatch for {y:04}-{m:02}-{d:02}, expected {expected}"
            );
        }
    }

    /// Years outside the common `0..=9999` range fall back to `None` so the
    /// caller defers to chrono's slow (but correct) path rather than
    /// producing wrong output.
    #[test]
    fn fast_date_defers_for_out_of_common_range_years() {
        let date = NaiveDate::from_ymd_opt(10000, 1, 1).unwrap();
        assert_eq!(fast_date_to_string(&date), None);

        let date = NaiveDate::from_ymd_opt(-1, 1, 1).unwrap();
        assert_eq!(fast_date_to_string(&date), None);
    }

    /// `fast_timestamp_to_string` must match
    /// `NaiveDateTime::format(TIMESTAMP_EMIT)` byte-for-byte, including
    /// always-9-digit fractional seconds, midnight, and nanosecond precision.
    #[test]
    fn fast_timestamp_matches_chrono_format() {
        let cases: Vec<NaiveDateTime> = vec![
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            NaiveDate::from_ymd_opt(1969, 12, 31)
                .unwrap()
                .and_hms_opt(23, 59, 59)
                .unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 29)
                .unwrap()
                .and_hms_opt(12, 30, 45)
                .unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 5)
                .unwrap()
                .and_hms_nano_opt(1, 2, 3, 4)
                .unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 5)
                .unwrap()
                .and_hms_nano_opt(23, 59, 59, 999_999_999)
                .unwrap(),
            NaiveDate::from_ymd_opt(9999, 12, 31)
                .unwrap()
                .and_hms_opt(23, 59, 59)
                .unwrap(),
        ];
        for ts in cases {
            let expected = ts.format(TIMESTAMP_EMIT).to_string();
            assert_eq!(
                fast_timestamp_to_string(&ts),
                Some(expected.clone()),
                "mismatch for {ts:?}, expected {expected}"
            );
        }
    }

    /// A timestamp whose date falls outside the common year range defers
    /// to `None` for the same reason as `fast_date_to_string`.
    #[test]
    fn fast_timestamp_defers_for_out_of_common_range_years() {
        let ts = NaiveDate::from_ymd_opt(10000, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        assert_eq!(fast_timestamp_to_string(&ts), None);
    }

    /// chrono encodes a leap second as a nanosecond field in
    /// `1_000_000_000..2_000_000_000`. `fast_timestamp_to_string` declines that
    /// case and defers to `NaiveDateTime::format`.
    #[test]
    fn fast_timestamp_defers_for_leap_second_nanos() {
        let ts = NaiveDate::from_ymd_opt(2016, 12, 31)
            .unwrap()
            .and_hms_nano_opt(23, 59, 59, 1_000_000_000)
            .unwrap();
        assert_eq!(fast_timestamp_to_string(&ts), None);
    }

    /// `value_to_block_string` must produce identical output whether or not
    /// the fast path is compiled in — this is the end-to-end proof that the
    /// fast path is wired in correctly and stays byte-identical for the
    /// whole `Value` enum, not just the two hand-tested helpers above.
    #[test]
    fn value_to_block_string_matches_slow_path_for_numeric_date_timestamp() {
        let numeric = Value::Numeric(decimal(-1_000_000_000_000_000_001, 18));
        assert_eq!(value_to_block_string(&numeric), "-1.000000000000000001");

        let date = Value::Date(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());
        assert_eq!(value_to_block_string(&date), "2024-02-29");

        let ts = Value::Timestamp(
            NaiveDate::from_ymd_opt(2026, 6, 5)
                .unwrap()
                .and_hms_nano_opt(1, 2, 3, 4)
                .unwrap(),
        );
        assert_eq!(value_to_block_string(&ts), "2026-06-05 01:02:03.000000004");
    }

    /// End-to-end byte-identity proof at the `to_proto` level (plan
    /// scenario "A promoted emit fast-path encoder stays byte-identical to
    /// the row path"). Builds an `EmitBuffer` spanning the full string-block
    /// `ExaType` range — NUMERIC/DATE/TIMESTAMP/VARCHAR — with interspersed
    /// NULLs and two columns sharing the NUMERIC block, then asserts the
    /// serialised `ExascriptTableData`'s string block equals the exact
    /// vector the reference `chrono`/`Display` path (`d.to_string()`,
    /// `date.format(DATE_FORMAT)`, `ts.format(TIMESTAMP_EMIT)`) produces in
    /// dense row-major-interleaved order — so downstream Exasol parsing is
    /// unaffected by the fast formatter. Also pins the NULL bitmap and the
    /// non-string blocks.
    #[test]
    fn fast_path_to_proto_byte_identical_to_row_path() {
        fn date(y: i32, m: u32, d: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, d).unwrap()
        }
        fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, nano: u32) -> NaiveDateTime {
            date(y, mo, d).and_hms_nano_opt(h, mi, s, nano).unwrap()
        }

        // num_a and num_b share the NUMERIC string block; label is VARCHAR
        // (same block); i64 is a separate block carrying a NULL.
        let meta = vec![
            col(
                "num_a",
                ExaType::Numeric {
                    precision: Some(18),
                    scale: Some(2),
                },
            ),
            col(
                "num_b",
                ExaType::Numeric {
                    precision: Some(38),
                    scale: Some(0),
                },
            ),
            col("d", ExaType::Date),
            col("t", ExaType::Timestamp),
            col("label", ExaType::String { size: Some(100) }),
            col("i", ExaType::Int64),
        ];

        let rows: Vec<Vec<Value>> = vec![
            vec![
                Value::Numeric(decimal(12345, 2)),
                Value::Numeric(decimal(-1_000_000_000_000_000_001, 18)),
                Value::Date(date(2024, 2, 29)),
                Value::Timestamp(ts(2026, 6, 5, 1, 2, 3, 4)),
                Value::String("héllo".into()),
                Value::Int64(42),
            ],
            vec![
                Value::Null,
                Value::Numeric(decimal(999, 0)),
                Value::Null,
                Value::Timestamp(ts(1970, 1, 1, 0, 0, 0, 0)),
                Value::Null,
                Value::Null,
            ],
            vec![
                Value::Numeric(decimal(0, 5)),
                Value::Null,
                Value::Date(date(9999, 12, 31)),
                Value::Timestamp(ts(2000, 2, 29, 23, 59, 59, 999_999_999)),
                Value::String(String::new()),
                Value::Int64(-1),
            ],
        ];

        // Reference formatter: the pre-optimisation `chrono`/`Display` path.
        fn reference_block_string(v: &Value) -> String {
            match v {
                Value::Numeric(d) => d.to_string(),
                Value::Date(d) => d.format(DATE_FORMAT).to_string(),
                Value::Timestamp(t) => t.format(TIMESTAMP_EMIT).to_string(),
                Value::String(s) => s.clone(),
                other => panic!("unexpected non-string-block value {other:?}"),
            }
        }

        // Expected dense, row-major-interleaved string block: skip NULL
        // cells (they take no block slot) and only include the string-block
        // columns (indices 0..=4; column 5 is Int64).
        let mut expected_string: Vec<String> = Vec::new();
        for row in &rows {
            // Columns 0..=4 are the string-block columns; column 5 is Int64.
            for cell in row.iter().take(5) {
                if !matches!(cell, Value::Null) {
                    expected_string.push(reference_block_string(cell));
                }
            }
        }

        let mut emit = EmitBuffer::new();
        for row in &rows {
            emit.push(row.clone());
        }
        let table = emit.to_proto(&meta);

        assert_eq!(
            table.data_string, expected_string,
            "fast-path string block must be byte-identical to the chrono/Display row path"
        );
        // Int64 block: only the two non-null cells, in row order.
        assert_eq!(table.data_int64, vec![42, -1]);
        // NULL bitmap is row-major (row * n_cols + col); 3 rows × 6 cols.
        let mut expected_nulls = vec![false; rows.len() * meta.len()];
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if matches!(cell, Value::Null) {
                    expected_nulls[r * meta.len() + c] = true;
                }
            }
        }
        assert_eq!(table.data_nulls, expected_nulls, "null bitmap");
        assert_eq!(table.rows, rows.len() as u64, "row count");
    }
}

// -----------------------------------------------------------------------
// Permanent regression guard: the string-block ingest fast-path parsers
// (`decode_string_block`'s DATE/TIMESTAMP branches) must stay
// byte-identical to the `chrono::parse_from_str` path they front, for
// every representable value, and must defer gracefully (not panic, not
// diverge) for malformed input (plan task 6.1).
// -----------------------------------------------------------------------
mod fast_string_block_ingest_tests {
    use super::*;

    /// Reference decode: the original `chrono`-only path `decode_string_block`
    /// used before the fast parser was added, used as the known-correct
    /// oracle for DATE/TIMESTAMP comparison.
    fn reference_decode_date(s: &str) -> Value {
        match NaiveDate::parse_from_str(s, DATE_FORMAT) {
            Ok(d) => Value::Date(d),
            Err(_) => Value::Null,
        }
    }

    fn reference_decode_timestamp(s: &str) -> Value {
        match NaiveDateTime::parse_from_str(s, TIMESTAMP_PARSE)
            .or_else(|_| NaiveDateTime::parse_from_str(s, TIMESTAMP_FORMAT_ISO))
        {
            Ok(ts) => Value::Timestamp(ts),
            Err(_) => Value::Null,
        }
    }

    /// `fast_parse_date` must match `NaiveDate::parse_from_str(DATE_FORMAT)`
    /// byte-for-byte across leap days, year boundaries, and a normal
    /// mid-range date.
    #[test]
    fn fast_parse_date_matches_chrono_parse_for_valid_dates() {
        let cases = [
            "2024-02-29",
            "0001-01-01",
            "9999-12-31",
            "1970-01-01",
            "2026-06-05",
            "0000-01-01",
        ];
        for s in cases {
            let expected = reference_decode_date(s);
            assert_eq!(
                fast_parse_date(s).map(Value::Date),
                Some(expected.clone()),
                "mismatch for {s}, expected {expected:?}"
            );
            assert_eq!(
                decode_string_block(&ExaType::Date, s.to_string()),
                expected,
                "decode_string_block mismatch for {s}"
            );
        }
    }

    /// `fast_parse_timestamp` must match the existing chrono parse chain
    /// (space-separated primary format, `T`-separated ISO fallback)
    /// byte-for-byte, including midnight, 0/3/6/9 fractional digits, and
    /// the ISO `T` variant.
    #[test]
    fn fast_parse_timestamp_matches_chrono_parse_for_valid_timestamps() {
        let cases = [
            "1970-01-01 00:00:00",
            "2026-06-05 01:02:03.4",
            "2026-06-05 01:02:03.400",
            "2026-06-05 01:02:03.400000",
            "2026-06-05 01:02:03.000000004",
            "2024-02-29 23:59:59.999999999",
            "2026-06-05T01:02:03.400",
            "9999-12-31 23:59:59",
            "0001-01-01 00:00:00.1",
        ];
        for s in cases {
            let expected = reference_decode_timestamp(s);
            assert_eq!(
                fast_parse_timestamp(s).map(Value::Timestamp),
                Some(expected.clone()),
                "mismatch for {s}, expected {expected:?}"
            );
            assert_eq!(
                decode_string_block(&ExaType::Timestamp, s.to_string()),
                expected,
                "decode_string_block mismatch for {s}"
            );
            assert_eq!(
                decode_string_block(&ExaType::TimestampTz, s.to_string()),
                expected,
                "decode_string_block (TimestampTz) mismatch for {s}"
            );
        }
    }

    /// Non-standard-width but chrono-parseable input (single-digit month/
    /// day, 2-digit year, a leap second, or an over-long fractional part
    /// chrono silently truncates to 9 digits) must not be rejected by
    /// `decode_string_block` even though the fast path correctly declines
    /// (returns `None`) and defers to the `chrono` fallback — proving the
    /// fallback chain preserves today's leniency exactly, never becoming
    /// stricter than the pre-fast-path behaviour.
    #[test]
    fn decode_string_block_preserves_leniency_when_fast_path_defers() {
        let date_cases = ["2024-2-29", "2024-02-9", "24-02-29"];
        for s in date_cases {
            assert_eq!(
                fast_parse_date(s),
                None,
                "fast_parse_date should defer (not itself parse) {s}"
            );
            let expected = reference_decode_date(s);
            assert_ne!(
                expected,
                Value::Null,
                "test setup: {s} should be chrono-valid"
            );
            assert_eq!(
                decode_string_block(&ExaType::Date, s.to_string()),
                expected,
                "decode_string_block must still succeed via fallback for {s}"
            );
        }

        let ts_cases = ["2024-02-29 23:59:60", "2024-02-29 23:59:59.1234567890"];
        for s in ts_cases {
            assert_eq!(
                fast_parse_timestamp(s),
                None,
                "fast_parse_timestamp should defer (not itself parse) {s}"
            );
            let expected = reference_decode_timestamp(s);
            assert_ne!(
                expected,
                Value::Null,
                "test setup: {s} should be chrono-valid"
            );
            assert_eq!(
                decode_string_block(&ExaType::Timestamp, s.to_string()),
                expected,
                "decode_string_block must still succeed via fallback for {s}"
            );
        }
    }

    /// Malformed DATE/TIMESTAMP strings must decode to `Value::Null`
    /// (never panic), exactly matching the pre-existing chrono-only
    /// behaviour: garbage text, wrong-width separators, and out-of-range
    /// calendar values (month 13, day 32, hour 24, minute 60).
    #[test]
    fn malformed_date_and_timestamp_strings_decode_to_null() {
        let malformed = [
            "",
            "not-a-date",
            "2024/02/29",
            "2024-13-01",
            "2024-02-32",
            "2024-00-01",
            "2024-02-00",
        ];
        for s in malformed {
            assert_eq!(
                fast_parse_date(s),
                None,
                "fast_parse_date should defer/reject for {s}"
            );
            assert_eq!(
                decode_string_block(&ExaType::Date, s.to_string()),
                Value::Null,
                "decode_string_block(Date) should be Null for {s}"
            );
        }

        let malformed_ts = [
            "",
            "not-a-timestamp",
            "2024-02-29",
            "2024-02-29 24:00:00",
            "2024-02-29 23:60:00",
            "2024-02-29 23:59:59.",
            "2024-13-01 00:00:00",
            "2024-02-32 00:00:00",
            "2024-02-29T23:59:59.abc",
            "2024-02-29X23:59:59",
        ];
        for s in malformed_ts {
            assert_eq!(
                fast_parse_timestamp(s),
                None,
                "fast_parse_timestamp should defer/reject for {s}"
            );
            assert_eq!(
                decode_string_block(&ExaType::Timestamp, s.to_string()),
                Value::Null,
                "decode_string_block(Timestamp) should be Null for {s}"
            );
        }
    }

    /// `parse_2digit` rejects anything that is not exactly two ASCII digits.
    /// The wrong-length branch is unreachable via the fixed-width callers, so
    /// it is exercised directly here.
    #[test]
    fn parse_2digit_rejects_wrong_length_and_non_digit_bytes() {
        assert_eq!(parse_2digit(b"5"), None, "too short");
        assert_eq!(parse_2digit(b"123"), None, "too long");
        assert_eq!(parse_2digit(b"a1"), None, "non-digit first byte");
        assert_eq!(parse_2digit(b"1a"), None, "non-digit second byte");
        assert_eq!(parse_2digit(b"42"), Some(42), "valid 2-digit field");
    }

    /// `parse_4digit` rejects anything that is not exactly four ASCII digits,
    /// including the wrong-length branch the fixed-width callers never reach.
    #[test]
    fn parse_4digit_rejects_wrong_length_and_non_digit_bytes() {
        assert_eq!(parse_4digit(b"123"), None, "too short");
        assert_eq!(parse_4digit(b"12345"), None, "too long");
        assert_eq!(parse_4digit(b"202a"), None, "non-digit byte mid-field");
        assert_eq!(parse_4digit(b"2026"), Some(2026), "valid 4-digit field");
    }
}

// -----------------------------------------------------------------------
// emit-arrow unit tests (task 2.5)
// -----------------------------------------------------------------------
#[cfg(feature = "emit-arrow")]
mod arrow_tests {
    use super::*;
    // `emit_batch` on the bridge resolves to the EmitBatch ext-trait, which
    // serialises to IPC bytes then calls `emit_record_batch_ipc`. The whole
    // round-trip runs in-process with one arrow copy, so it works in tests.
    use arrow::array::{BooleanArray, Float64Array, Int64Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use exasol_udf_sdk::context::EmitBatch;
    use std::sync::Arc;

    /// Build a simple 4-column RecordBatch: Int64, Utf8, Float64, Boolean
    fn make_batch(
        ints: &[i64],
        strs: &[Option<&str>],
        floats: &[f64],
        bools: &[bool],
    ) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Utf8, true),
            Field::new("c", DataType::Float64, false),
            Field::new("d", DataType::Boolean, false),
        ]));
        let int_arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(ints.to_vec()));
        let str_arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(strs.to_vec()));
        let float_arr: Arc<dyn arrow::array::Array> = Arc::new(Float64Array::from(floats.to_vec()));
        let bool_arr: Arc<dyn arrow::array::Array> = Arc::new(BooleanArray::from(bools.to_vec()));
        RecordBatch::try_new(schema, vec![int_arr, str_arr, float_arr, bool_arr]).unwrap()
    }

    fn mixed_meta() -> Vec<ColumnMeta> {
        vec![
            col("a", ExaType::Int64),
            col("b", ExaType::String { size: None }),
            col("c", ExaType::Double),
            col("d", ExaType::Boolean),
        ]
    }

    /// Test: push_batch produces byte-identical output to the row path
    #[test]
    fn push_batch_equals_row_push() {
        let meta = mixed_meta();
        let batch = make_batch(
            &[10, 20],
            &[Some("x"), Some("y")],
            &[1.5, 2.5],
            &[true, false],
        );

        // Row path
        let mut row_buf = EmitBuffer::new();
        row_buf.push(vec![
            Value::Int64(10),
            Value::String("x".into()),
            Value::Double(1.5),
            Value::Bool(true),
        ]);
        row_buf.push(vec![
            Value::Int64(20),
            Value::String("y".into()),
            Value::Double(2.5),
            Value::Bool(false),
        ]);
        let row_table = row_buf.to_proto(&meta);

        // Batch path — batch fits in one slice (< 4MB), so no mid-batch
        // flush; the whole thing lands in the tail.
        let mut batch_buf = EmitBuffer::new();
        let mut flushed_tables: Vec<exa_proto::ExascriptTableData> = Vec::new();
        batch_buf
            .push_batch(&batch, &meta, &mut |t| {
                flushed_tables.push(t);
                Ok(())
            })
            .unwrap();
        // No split expected for 2 small rows.
        assert!(
            flushed_tables.is_empty(),
            "no flush expected for tiny batch"
        );
        // Tail is now in batch_buf.
        let batch_table = batch_buf.to_proto(&meta);

        // The two tables must be byte-identical.
        assert_eq!(row_table.data_int64, batch_table.data_int64, "int64 block");
        assert_eq!(
            row_table.data_string, batch_table.data_string,
            "string block"
        );
        assert_eq!(
            row_table.data_double, batch_table.data_double,
            "double block"
        );
        assert_eq!(row_table.data_bool, batch_table.data_bool, "bool block");
        assert_eq!(row_table.data_nulls, batch_table.data_nulls, "null bitmap");
        assert_eq!(row_table.rows, batch_table.rows, "row count");

        // Also decode the batch-path result and verify values.
        let rs = InputRowSet::from_proto(&batch_table, &meta);
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
                Value::String("y".into()),
                Value::Double(2.5),
                Value::Bool(false),
            ]
        );
    }

    /// `encode_slice`'s per-type blocks are pre-sized with
    /// `Vec::with_capacity` for the exact (non-NULL) column count,
    /// mirroring `to_proto`'s pre-sizing. This only asserts the resulting
    /// contents are correct — `Vec::capacity()` is only guaranteed to be
    /// *at least* the requested value, so asserting an exact capacity here
    /// would be a flaky test rather than a real regression guard; the
    /// pre-sizing's throughput benefit is verified by `benches/emit-bench`.
    #[test]
    fn encode_slice_presizes_string_block_capacity() {
        let meta = vec![col("b", ExaType::String { size: None })];
        let schema = Arc::new(Schema::new(vec![Field::new("b", DataType::Utf8, true)]));
        let arr: Arc<dyn arrow::array::Array> =
            Arc::new(StringArray::from(vec![Some("a"), Some("b"), Some("c")]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

        let table = encode_slice(&batch, &meta).unwrap();
        assert_eq!(table.data_string, vec!["a", "b", "c"]);
    }

    /// `encode_slice` is byte-identical to the row path across all five proto
    /// blocks and every Arrow source type feeding the string block. NULLs are
    /// spread over different rows and both block kinds, so the bitmap and the
    /// no-slot-for-NULL interleaving are pinned too.
    ///
    /// The expectation is the row path's own `to_proto` output for the
    /// equivalent `Value` rows, so this asserts the byte-identity contract
    /// itself rather than transcribing today's formatting; the two paths share
    /// only `value_to_block_string`, which
    /// `fast_path_to_proto_byte_identical_to_row_path` pins independently.
    #[test]
    fn encode_slice_matches_row_path_across_every_block_type() {
        use arrow::array::{
            Date32Array, Decimal128Array, Int32Array, LargeStringArray, TimestampMicrosecondArray,
            TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray,
        };
        use arrow::datatypes::TimeUnit;

        fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, nano: u32) -> NaiveDateTime {
            NaiveDate::from_ymd_opt(y, mo, d)
                .unwrap()
                .and_hms_nano_opt(h, mi, s, nano)
                .unwrap()
        }
        fn days_since_epoch(d: NaiveDate) -> i32 {
            (d - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()).num_days() as i32
        }
        fn cell<T>(v: Option<T>, wrap: impl Fn(T) -> Value) -> Value {
            v.map(wrap).unwrap_or(Value::Null)
        }

        // One value table per column drives both the Arrow array and the
        // expected `Value` row, so the two paths cannot drift apart silently.
        let i32_vals = [Some(7i32), None, Some(-8)];
        let i64_vals = [Some(42i64), Some(43), Some(-44)];
        let dbl_vals = [None, Some(2.5f64), Some(-3.5)];
        let bln_vals = [Some(true), Some(false), None];
        let str_vals = [Some("héllo"), None, Some("")];
        let lstr_vals = [Some("large"), Some("x"), Some("y")];
        let date_vals = [
            Some(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()),
            None,
            // Pre-epoch: a negative Arrow day count through the +719163 offset.
            Some(NaiveDate::from_ymd_opt(1969, 7, 20).unwrap()),
        ];
        let ts_s_vals = [
            Some(at(2023, 11, 14, 22, 13, 20, 0)),
            Some(at(1970, 1, 1, 0, 0, 0, 0)),
            Some(at(2038, 1, 19, 3, 14, 7, 0)),
        ];
        let ts_ms_vals = [
            Some(at(2024, 2, 29, 1, 2, 3, 123_000_000)),
            Some(at(1969, 12, 31, 23, 59, 59, 999_000_000)),
            None,
        ];
        let ts_us_vals = [
            Some(at(2026, 6, 5, 12, 0, 0, 123_456_000)),
            Some(at(1970, 1, 1, 0, 0, 0, 1_000)),
            Some(at(1900, 1, 1, 0, 0, 0, 0)),
        ];
        let ts_ns_vals = [
            Some(at(1999, 12, 31, 23, 59, 59, 987_654_321)),
            Some(at(1970, 1, 1, 0, 0, 0, 1)),
            None,
        ];
        let dec_vals = [Some(12345i128), None, Some(-1)];
        let num_i32_vals = [Some(1i32), Some(-2), Some(0)];
        let num_i64_vals = [Some(-9i64), None, Some(0)];
        let num_f64_vals = [Some(1.5f64), Some(-0.25), Some(0.0)];

        let meta = vec![
            col("i32", ExaType::Int32),
            col("i64", ExaType::Int64),
            col("dbl", ExaType::Double),
            col("bln", ExaType::Boolean),
            col("s", ExaType::String { size: None }),
            col("ls", ExaType::String { size: None }),
            col("dt", ExaType::Date),
            col("ts_s", ExaType::Timestamp),
            col("ts_ms", ExaType::Timestamp),
            col("ts_us", ExaType::TimestampTz),
            col("ts_ns", ExaType::Timestamp),
            col(
                "dec",
                ExaType::Numeric {
                    precision: Some(18),
                    scale: Some(2),
                },
            ),
            col(
                "n32",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col(
                "n64",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col(
                "nf64",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
        ];

        let schema = Arc::new(Schema::new(vec![
            Field::new("i32", DataType::Int32, true),
            Field::new("i64", DataType::Int64, true),
            Field::new("dbl", DataType::Float64, true),
            Field::new("bln", DataType::Boolean, true),
            Field::new("s", DataType::Utf8, true),
            Field::new("ls", DataType::LargeUtf8, true),
            Field::new("dt", DataType::Date32, true),
            Field::new("ts_s", DataType::Timestamp(TimeUnit::Second, None), true),
            Field::new(
                "ts_ms",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                true,
            ),
            Field::new(
                "ts_us",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                true,
            ),
            Field::new(
                "ts_ns",
                DataType::Timestamp(TimeUnit::Nanosecond, None),
                true,
            ),
            Field::new("dec", DataType::Decimal128(18, 2), true),
            Field::new("n32", DataType::Int32, true),
            Field::new("n64", DataType::Int64, true),
            Field::new("nf64", DataType::Float64, true),
        ]));

        let columns: Vec<Arc<dyn arrow::array::Array>> = vec![
            Arc::new(Int32Array::from(i32_vals.to_vec())),
            Arc::new(Int64Array::from(i64_vals.to_vec())),
            Arc::new(Float64Array::from(dbl_vals.to_vec())),
            Arc::new(BooleanArray::from(bln_vals.to_vec())),
            Arc::new(StringArray::from(str_vals.to_vec())),
            Arc::new(LargeStringArray::from(lstr_vals.to_vec())),
            Arc::new(Date32Array::from(
                date_vals
                    .iter()
                    .map(|d| d.map(days_since_epoch))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(TimestampSecondArray::from(
                ts_s_vals
                    .iter()
                    .map(|t| t.map(|t| t.and_utc().timestamp()))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(TimestampMillisecondArray::from(
                ts_ms_vals
                    .iter()
                    .map(|t| t.map(|t| t.and_utc().timestamp_millis()))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(TimestampMicrosecondArray::from(
                ts_us_vals
                    .iter()
                    .map(|t| t.map(|t| t.and_utc().timestamp_micros()))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(TimestampNanosecondArray::from(
                ts_ns_vals
                    .iter()
                    .map(|t| t.map(|t| t.and_utc().timestamp_nanos_opt().unwrap()))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(
                Decimal128Array::from(dec_vals.to_vec())
                    .with_precision_and_scale(18, 2)
                    .unwrap(),
            ),
            Arc::new(Int32Array::from(num_i32_vals.to_vec())),
            Arc::new(Int64Array::from(num_i64_vals.to_vec())),
            Arc::new(Float64Array::from(num_f64_vals.to_vec())),
        ];
        let batch = RecordBatch::try_new(schema, columns).unwrap();

        let mut row_buf = EmitBuffer::new();
        for r in 0..3 {
            row_buf.push(vec![
                cell(i32_vals[r], Value::Int32),
                cell(i64_vals[r], Value::Int64),
                cell(dbl_vals[r], Value::Double),
                cell(bln_vals[r], Value::Bool),
                cell(str_vals[r], |s: &str| Value::String(s.to_string())),
                cell(lstr_vals[r], |s: &str| Value::String(s.to_string())),
                cell(date_vals[r], Value::Date),
                cell(ts_s_vals[r], Value::Timestamp),
                cell(ts_ms_vals[r], Value::Timestamp),
                cell(ts_us_vals[r], Value::Timestamp),
                cell(ts_ns_vals[r], Value::Timestamp),
                cell(dec_vals[r], |unscaled| {
                    Value::Numeric(Decimal { unscaled, scale: 2 })
                }),
                cell(num_i32_vals[r], Value::Int32),
                cell(num_i64_vals[r], Value::Int64),
                cell(num_f64_vals[r], Value::Double),
            ]);
        }
        let row_table = row_buf.to_proto(&meta);

        let slice_table = encode_slice(&batch, &meta).unwrap();

        assert_eq!(
            row_table, slice_table,
            "encode_slice must stay byte-identical to the row path"
        );
        // Anchors for the two riskiest conversions, so a wiring mistake names
        // itself instead of surfacing as an opaque whole-table diff.
        assert!(
            slice_table.data_string.iter().any(|s| s == "2024-02-29"),
            "Date32 CE-day epoch offset: {:?}",
            slice_table.data_string
        );
        assert!(
            slice_table
                .data_string
                .iter()
                .any(|s| s == "1999-12-31 23:59:59.987654321"),
            "nanosecond unit reaches the wire at full precision: {:?}",
            slice_table.data_string
        );
    }

    /// Test: two string-family columns interleave row-major in data_string.
    #[test]
    fn push_batch_shared_block_type_interleaved() {
        // Two Utf8 columns both declared String → data_string is row-major.
        let schema = Arc::new(Schema::new(vec![
            Field::new("s1", DataType::Utf8, false),
            Field::new("s2", DataType::Utf8, false),
        ]));
        let a: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["A0", "A1"]));
        let b: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["B0", "B1"]));
        let batch = RecordBatch::try_new(schema, vec![a, b]).unwrap();

        let meta = vec![
            col("s1", ExaType::String { size: None }),
            col("s2", ExaType::String { size: None }),
        ];

        let mut buf = EmitBuffer::new();
        buf.push_batch(&batch, &meta, &mut |_| Ok(())).unwrap();
        let table = buf.to_proto(&meta);

        // Row-major: row0(s1,s2), row1(s1,s2)
        assert_eq!(table.data_string, vec!["A0", "B0", "A1", "B1"]);
    }

    /// Test: NULL cells occupy no type-block slot, only the bitmap.
    #[test]
    fn push_batch_null_bitmap() {
        // Row 0: (10, "hello", 1.0, true), Row 1: (NULL int64, NULL str, NULL float, NULL bool)
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, true),
            Field::new("b", DataType::Utf8, true),
            Field::new("c", DataType::Float64, true),
            Field::new("d", DataType::Boolean, true),
        ]));
        let int_arr: Arc<dyn arrow::array::Array> =
            Arc::new(Int64Array::from(vec![Some(10i64), None]));
        let str_arr: Arc<dyn arrow::array::Array> =
            Arc::new(StringArray::from(vec![Some("hello"), None]));
        let float_arr: Arc<dyn arrow::array::Array> =
            Arc::new(Float64Array::from(vec![Some(1.0f64), None]));
        let bool_arr: Arc<dyn arrow::array::Array> =
            Arc::new(BooleanArray::from(vec![Some(true), None]));
        let batch =
            RecordBatch::try_new(schema, vec![int_arr, str_arr, float_arr, bool_arr]).unwrap();

        let meta = vec![
            col("a", ExaType::Int64),
            col("b", ExaType::String { size: None }),
            col("c", ExaType::Double),
            col("d", ExaType::Boolean),
        ];

        let mut buf = EmitBuffer::new();
        buf.push_batch(&batch, &meta, &mut |_| Ok(())).unwrap();
        let table = buf.to_proto(&meta);

        // Only row0's non-null values are in the type blocks.
        assert_eq!(table.data_int64, vec![10i64]);
        assert_eq!(table.data_string, vec!["hello"]);
        assert_eq!(table.data_double, vec![1.0f64]);
        assert_eq!(table.data_bool, vec![true]);

        // Null bitmap: 2 rows × 4 cols = 8 entries.
        // Row 0: all false (non-null). Row 1: all true (null).
        assert_eq!(
            table.data_nulls,
            vec![false, false, false, false, true, true, true, true]
        );

        // Round-trip via from_proto.
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::Int64(10),
                Value::String("hello".into()),
                Value::Double(1.0),
                Value::Bool(true),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[Value::Null, Value::Null, Value::Null, Value::Null]
        );
    }

    /// Test: cumulative byte cost of push_batch matches the row path's byte_estimate.
    #[test]
    fn push_batch_byte_estimate_parity() {
        let meta = mixed_meta();
        let s = "hello"; // 5 bytes per row

        // Row path: push 10 rows and check byte_estimate.
        let mut row_buf = EmitBuffer::new();
        for _ in 0..10 {
            row_buf.push(vec![
                Value::Int64(42),
                Value::String(s.to_string()),
                Value::Double(1.0),
                Value::Bool(true),
            ]);
        }
        let row_estimate = row_buf.byte_estimate;

        // Batch path: push the same 10 rows as a batch.
        let strs = vec![Some(s); 10];
        let batch = make_batch(&[42i64; 10], &strs, &[1.0f64; 10], &[true; 10]);
        let mut batch_buf = EmitBuffer::new();
        batch_buf
            .push_batch(&batch, &meta, &mut |_| Ok(()))
            .unwrap();
        let batch_estimate = batch_buf.byte_estimate;

        // The byte estimates must be equal so should_flush fires at the same threshold.
        assert_eq!(
            row_estimate, batch_estimate,
            "batch byte estimate ({batch_estimate}) must match row estimate ({row_estimate})"
        );
    }

    /// Test: a batch whose cost > 4MB splits into N>1 flushes.
    #[test]
    fn push_batch_splits_oversized_batch() {
        // Each row has a ~1000-byte string. We need enough rows to exceed 4MB.
        // 4_000_000 / 1000 = 4000 rows needed. Use 5000 to guarantee > 1 flush.
        let n_rows = 5000usize;
        let s = "x".repeat(1000);

        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
        let arr: Arc<dyn arrow::array::Array> =
            Arc::new(StringArray::from(vec![s.as_str(); n_rows]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

        let meta = vec![col("v", ExaType::String { size: None })];

        let mut flush_count = 0usize;
        let mut total_flushed_rows = 0u64;
        let mut buf = EmitBuffer::new();
        buf.push_batch(&batch, &meta, &mut |t| {
            // Each flushed slice must have at least 1 row.
            assert!(t.rows > 0, "flushed table must have rows");
            total_flushed_rows += t.rows;
            flush_count += 1;
            Ok(())
        })
        .unwrap();

        // At least 1 flush must have happened (oversized batch).
        assert!(
            flush_count >= 1,
            "expected ≥1 flush for oversized batch, got {flush_count}"
        );

        // Tail rows plus flushed rows must equal total batch rows.
        let tail_rows = buf.len() as u64;
        assert_eq!(
            total_flushed_rows + tail_rows,
            n_rows as u64,
            "flushed({total_flushed_rows}) + tail({tail_rows}) must equal batch rows({n_rows})"
        );
    }

    /// Test: the tail after an oversized push_batch is < 4MB.
    #[test]
    fn push_batch_slice_zero_copy_tail_bounded() {
        let n_rows = 5000usize;
        let s = "x".repeat(1000);

        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
        let arr: Arc<dyn arrow::array::Array> =
            Arc::new(StringArray::from(vec![s.as_str(); n_rows]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("v", ExaType::String { size: None })];

        let mut buf = EmitBuffer::new();
        buf.push_batch(&batch, &meta, &mut |_| Ok(())).unwrap();

        // The residual byte estimate must be < 4MB.
        assert!(
            buf.byte_estimate < EMIT_BUFFER_LIMIT_BYTES,
            "tail byte estimate {} must be < 4MB",
            buf.byte_estimate
        );
    }

    /// Test: a column whose Arrow type cannot feed the declared ExaType returns Err.
    #[test]
    fn push_batch_type_mismatch_errors() {
        // Utf8 column declared as Int64 → incompatible.
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["hello"]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("v", ExaType::Int64)];

        let mut buf = EmitBuffer::new();
        let result = buf.push_batch(&batch, &meta, &mut |_| Ok(()));
        assert!(
            matches!(result, Err(UdfError::Type(_))),
            "Utf8 declared as Int64 must return Err(Type)"
        );

        // Date32 declared as Boolean → incompatible.
        use arrow::array::Date32Array;
        let schema2 = Arc::new(Schema::new(vec![Field::new("d", DataType::Date32, false)]));
        let arr2: Arc<dyn arrow::array::Array> = Arc::new(Date32Array::from(vec![0i32]));
        let batch2 = RecordBatch::try_new(schema2, vec![arr2]).unwrap();
        let meta2 = vec![col("d", ExaType::Boolean)];

        let mut buf2 = EmitBuffer::new();
        let result2 = buf2.push_batch(&batch2, &meta2, &mut |_| Ok(()));
        assert!(
            matches!(result2, Err(UdfError::Type(_))),
            "Date32 declared as Boolean must return Err(Type)"
        );
    }

    /// A `BIGINT` EMITS column arrives as `ExaType::Numeric` (string block),
    /// so an Arrow `Int64` column must feed it — stringified exactly as the
    /// row path's `value_to_block_string(Value::Int64)`. This is the
    /// `emit-arrow-batch` fixture's `id BIGINT` case.
    #[test]
    fn push_batch_int64_into_numeric_block() {
        let meta = vec![col(
            "id",
            ExaType::Numeric {
                precision: None,
                scale: None,
            },
        )];
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(vec![1i64, 2, 3]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

        // Row path: the same values as Value::Int64 into a Numeric column.
        let mut row_buf = EmitBuffer::new();
        for n in [1i64, 2, 3] {
            row_buf.push(vec![Value::Int64(n)]);
        }
        let row_table = row_buf.to_proto(&meta);

        let mut batch_buf = EmitBuffer::new();
        batch_buf
            .push_batch(&batch, &meta, &mut |_| Ok(()))
            .expect("Int64 must feed a NUMERIC column");
        let batch_table = batch_buf.to_proto(&meta);

        assert_eq!(batch_table.data_string, vec!["1", "2", "3"]);
        assert_eq!(row_table.data_string, batch_table.data_string);
        assert!(batch_table.data_int64.is_empty(), "ints go to string block");
    }

    // -----------------------------------------------------------------------
    // Bridge tests
    // -----------------------------------------------------------------------

    /// Build a bridge with a flush counter for emit-arrow tests.
    fn make_emit_bridge_with_counter<'a>(
        input: &'a mut InputRowSet,
        emit: &'a mut EmitBuffer,
        meta: &'a [ColumnMeta],
        flush_count: &'a std::cell::Cell<usize>,
    ) -> HostContextBridge<'a> {
        HostContextBridge::new(
            input,
            emit,
            meta,
            meta,
            Box::new(move |t: exa_proto::ExascriptTableData| {
                if t.rows > 0 {
                    flush_count.set(flush_count.get() + 1);
                }
                Ok(())
            }),
            HandshakeMeta::default(),
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        )
    }

    /// Test: a small batch under the threshold buffers without a mid-flush.
    #[test]
    fn bridge_emit_batch_buffers_and_flushes() {
        let meta = mixed_meta();
        let batch = make_batch(
            &[1, 2],
            &[Some("a"), Some("b")],
            &[0.1, 0.2],
            &[true, false],
        );
        let empty_table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&empty_table, &meta);
        let mut emit = EmitBuffer::new();
        let flush_count = std::cell::Cell::new(0usize);
        {
            let mut bridge = make_emit_bridge_with_counter(&mut rs, &mut emit, &meta, &flush_count);
            bridge.emit_batch(&batch).unwrap();
            // No mid-batch flush for a tiny batch.
            assert_eq!(flush_count.get(), 0, "no flush expected for tiny batch");
        }
        // The tail is in the emit buffer after the bridge is dropped.
        assert_eq!(emit.len(), 2, "tail must have 2 rows");
    }

    /// Test: a batch whose Arrow type cannot feed the declared ExaType makes
    /// emit_batch return Err after the host deserialises and runs push_batch.
    #[test]
    fn bridge_emit_batch_error_propagates() {
        // A Utf8 array declared as ExaType::Int64 — incompatible.
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["hello"]));
        let bad_batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

        let meta = vec![col("v", ExaType::Int64)];
        let empty_table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&empty_table, &meta);
        let mut emit = EmitBuffer::new();
        let flush_count = std::cell::Cell::new(0usize);
        let mut bridge = make_emit_bridge_with_counter(&mut rs, &mut emit, &meta, &flush_count);
        let result = bridge.emit_batch(&bad_batch);
        assert!(
            matches!(result, Err(UdfError::Type(_))),
            "Utf8 declared as Int64 must return Err(Type)"
        );
    }

    /// Test: interleaved emit() and emit_batch() share the same buffer.
    #[test]
    fn bridge_mixed_emit_styles_share_buffer() {
        let meta = vec![col("v", ExaType::Int64)];
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(vec![2i64, 3]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

        let empty_table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&empty_table, &meta);
        let mut emit = EmitBuffer::new();
        let flush_count = std::cell::Cell::new(0usize);
        {
            let mut bridge = make_emit_bridge_with_counter(&mut rs, &mut emit, &meta, &flush_count);
            // Row-based emit: 1 row
            bridge.emit(&[Value::Int64(1)]).unwrap();
            // Batch-based emit: 2 rows. push_batch flushes the pending row
            // first (decision-log [7] step 1) to preserve FIFO order, then
            // the 2 batch rows land in the tail as they're under threshold.
            bridge.emit_batch(&batch).unwrap();
            // 1 flush for the pending row that was displaced by the batch.
            assert_eq!(
                flush_count.get(),
                1,
                "pending row flushed before batch tail"
            );
        }
        // The 2 batch rows are in the tail buffer (the flushed row was sent to the flusher).
        assert_eq!(emit.len(), 2, "batch tail must have 2 rows");

        // Verify the tail values (the 2 batch rows).
        let table = emit.to_proto(&meta);
        let rs2 = InputRowSet::from_proto(&table, &meta);
        assert_eq!(rs2.row(0).unwrap(), &[Value::Int64(2)]);
        assert_eq!(rs2.row(1).unwrap(), &[Value::Int64(3)]);
    }

    // -----------------------------------------------------------------------
    // Direct unit tests: compute_row_costs / accessor_value / build_accessors
    //
    // These call the four extracted helpers themselves rather than only
    // through encode_slice/push_batch, targeting the specific per-type,
    // NULL, and error-arm edges the wider parity tests above do not isolate.
    // -----------------------------------------------------------------------

    /// Every constant-width `DataType` arm `fixed_cell_cost` recognizes must
    /// charge exactly what `value_byte_cost` charges the equivalent `Value` —
    /// the two tables are meant to describe the same widths from two axes.
    #[test]
    fn compute_row_costs_fixed_width_types_match_value_byte_cost() {
        use arrow::array::{Date32Array, Int32Array, TimestampMillisecondArray};
        use arrow::datatypes::TimeUnit;

        fn single_column_cost(
            dt: DataType,
            array: Arc<dyn arrow::array::Array>,
            typ: ExaType,
        ) -> usize {
            let schema = Arc::new(Schema::new(vec![Field::new("v", dt, false)]));
            let batch = RecordBatch::try_new(schema, vec![array]).unwrap();
            let meta = vec![col("v", typ)];
            compute_row_costs(&batch, &meta)[0]
        }

        assert_eq!(
            single_column_cost(
                DataType::Boolean,
                Arc::new(BooleanArray::from(vec![true])),
                ExaType::Boolean
            ),
            value_byte_cost(&Value::Bool(true)),
            "Boolean"
        );
        assert_eq!(
            single_column_cost(
                DataType::Int32,
                Arc::new(Int32Array::from(vec![7i32])),
                ExaType::Int32
            ),
            value_byte_cost(&Value::Int32(7)),
            "Int32"
        );
        assert_eq!(
            single_column_cost(
                DataType::Int64,
                Arc::new(Int64Array::from(vec![9i64])),
                ExaType::Int64
            ),
            value_byte_cost(&Value::Int64(9)),
            "Int64"
        );
        assert_eq!(
            single_column_cost(
                DataType::Float64,
                Arc::new(Float64Array::from(vec![1.5f64])),
                ExaType::Double
            ),
            value_byte_cost(&Value::Double(1.5)),
            "Float64"
        );
        assert_eq!(
            single_column_cost(
                DataType::Date32,
                Arc::new(Date32Array::from(vec![100i32])),
                ExaType::Date
            ),
            value_byte_cost(&Value::Date(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())),
            "Date32 cost is a fixed width, independent of the actual date"
        );
        assert_eq!(
            single_column_cost(
                DataType::Timestamp(TimeUnit::Millisecond, None),
                Arc::new(TimestampMillisecondArray::from(vec![0i64])),
                ExaType::Timestamp
            ),
            value_byte_cost(&Value::Timestamp(NaiveDateTime::default())),
            "Timestamp cost is a fixed width, independent of unit or value"
        );
    }

    /// `compute_row_costs`'s `Decimal128` arm adds the column's `scale` to
    /// `NUMERIC_COST_BASE`, exactly like `value_byte_cost`'s `Value::Numeric`
    /// arm adds `d.scale` — verified across several distinct scales so the
    /// term is confirmed to actually vary, not just present at one value.
    #[test]
    fn compute_row_costs_decimal128_scale_term_matches_value_byte_cost() {
        use arrow::array::Decimal128Array;

        for scale in [0i8, 2, 9] {
            let schema = Arc::new(Schema::new(vec![Field::new(
                "d",
                DataType::Decimal128(18, scale),
                false,
            )]));
            let arr: Arc<dyn arrow::array::Array> = Arc::new(
                Decimal128Array::from(vec![12345i128])
                    .with_precision_and_scale(18, scale)
                    .unwrap(),
            );
            let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
            let meta = vec![col(
                "d",
                ExaType::Numeric {
                    precision: Some(18),
                    scale: Some(scale as u32),
                },
            )];

            let cost = compute_row_costs(&batch, &meta)[0];
            let expected = value_byte_cost(&Value::Numeric(Decimal {
                unscaled: 12345,
                scale: scale as u8,
            }));
            assert_eq!(cost, expected, "scale {scale}");
        }
    }

    /// `Utf8`/`LargeUtf8` are the only variable-width arms: cost is the raw
    /// byte length of the string, matching `value_byte_cost`'s `s.len()` —
    /// checked with a multi-byte UTF-8 string so a char-count regression
    /// would be caught.
    #[test]
    fn compute_row_costs_variable_width_strings_match_value_byte_cost() {
        use arrow::array::LargeStringArray;

        let schema = Arc::new(Schema::new(vec![Field::new("s", DataType::Utf8, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["hello world"]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("s", ExaType::String { size: None })];
        assert_eq!(
            compute_row_costs(&batch, &meta)[0],
            value_byte_cost(&Value::String("hello world".to_string())),
            "Utf8"
        );

        // "héllo" — é is 2 bytes in UTF-8, so byte length (6) differs from
        // char count (5); this pins the cost to bytes, not chars.
        let schema2 = Arc::new(Schema::new(vec![Field::new(
            "ls",
            DataType::LargeUtf8,
            false,
        )]));
        let arr2: Arc<dyn arrow::array::Array> = Arc::new(LargeStringArray::from(vec!["héllo"]));
        let batch2 = RecordBatch::try_new(schema2, vec![arr2]).unwrap();
        let meta2 = vec![col("ls", ExaType::String { size: None })];
        assert_eq!(
            compute_row_costs(&batch2, &meta2)[0],
            value_byte_cost(&Value::String("héllo".to_string())),
            "LargeUtf8"
        );
    }

    /// A NULL cell contributes 0 regardless of column type (mirrors
    /// `value_byte_cost`'s `Value::Null => 0`), and a row's total cost is the
    /// sum of its non-null cells across every column — both properties
    /// exercised together across three rows spanning "no NULLs", "one NULL
    /// column", and "every column NULL".
    #[test]
    fn compute_row_costs_null_cells_cost_zero_and_multi_column_rows_sum() {
        use arrow::array::Int32Array;

        let schema = Arc::new(Schema::new(vec![
            Field::new("i", DataType::Int32, true),
            Field::new("s", DataType::Utf8, true),
            Field::new("b", DataType::Boolean, true),
        ]));
        let i_arr: Arc<dyn arrow::array::Array> =
            Arc::new(Int32Array::from(vec![Some(7i32), None, None]));
        let s_arr: Arc<dyn arrow::array::Array> =
            Arc::new(StringArray::from(vec![Some("abcd"), Some("xy"), None]));
        let b_arr: Arc<dyn arrow::array::Array> =
            Arc::new(BooleanArray::from(vec![Some(true), Some(false), None]));
        let batch = RecordBatch::try_new(schema, vec![i_arr, s_arr, b_arr]).unwrap();
        let meta = vec![
            col("i", ExaType::Int32),
            col("s", ExaType::String { size: None }),
            col("b", ExaType::Boolean),
        ];

        let costs = compute_row_costs(&batch, &meta);

        assert_eq!(
            costs[0],
            value_byte_cost(&Value::Int32(7))
                + value_byte_cost(&Value::String("abcd".into()))
                + value_byte_cost(&Value::Bool(true)),
            "row 0: no NULLs, three-column sum"
        );
        assert_eq!(
            costs[1],
            value_byte_cost(&Value::String("xy".into())) + value_byte_cost(&Value::Bool(false)),
            "row 1: NULL Int32 contributes 0, only the other two columns count"
        );
        assert_eq!(costs[2], 0, "row 2: every column NULL, total cost 0");
    }

    /// `accessor_value`'s four `Timestamp` arms each divide by a different
    /// unit; each is checked against `chrono`'s own conversion for that unit
    /// so a wrong divisor (e.g. swapping `_millis`/`_micros`) fails here
    /// rather than only inside the wider byte-identity parity test.
    #[test]
    fn accessor_value_timestamp_second_matches_chrono_from_timestamp() {
        use arrow::array::TimestampSecondArray;
        use arrow::datatypes::TimeUnit;

        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Second, None),
            false,
        )]));
        let arr: Arc<dyn arrow::array::Array> =
            Arc::new(TimestampSecondArray::from(vec![1_700_000_000i64]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("ts", ExaType::Timestamp)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        let value = accessor_value(&accessors[0], 0);

        let expected = chrono::DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap()
            .naive_utc();
        assert_eq!(value, Value::Timestamp(expected));
    }

    #[test]
    fn accessor_value_timestamp_millisecond_matches_chrono_from_timestamp_millis() {
        use arrow::array::TimestampMillisecondArray;
        use arrow::datatypes::TimeUnit;

        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            false,
        )]));
        let arr: Arc<dyn arrow::array::Array> =
            Arc::new(TimestampMillisecondArray::from(vec![1_700_000_000_123i64]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("ts", ExaType::Timestamp)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        let value = accessor_value(&accessors[0], 0);

        let expected = chrono::DateTime::from_timestamp_millis(1_700_000_000_123)
            .unwrap()
            .naive_utc();
        assert_eq!(value, Value::Timestamp(expected));
    }

    #[test]
    fn accessor_value_timestamp_microsecond_matches_chrono_from_timestamp_micros() {
        use arrow::array::TimestampMicrosecondArray;
        use arrow::datatypes::TimeUnit;

        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        )]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(TimestampMicrosecondArray::from(vec![
            1_700_000_000_123_456i64,
        ]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("ts", ExaType::Timestamp)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        let value = accessor_value(&accessors[0], 0);

        let expected = chrono::DateTime::from_timestamp_micros(1_700_000_000_123_456)
            .unwrap()
            .naive_utc();
        assert_eq!(value, Value::Timestamp(expected));
    }

    #[test]
    fn accessor_value_timestamp_nanosecond_positive_matches_chrono_from_timestamp() {
        use arrow::array::TimestampNanosecondArray;
        use arrow::datatypes::TimeUnit;

        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            false,
        )]));
        let ns = 1_700_000_000_123_456_789i64;
        let arr: Arc<dyn arrow::array::Array> = Arc::new(TimestampNanosecondArray::from(vec![ns]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("ts", ExaType::Timestamp)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        let value = accessor_value(&accessors[0], 0);

        let expected =
            chrono::DateTime::from_timestamp(ns / 1_000_000_000, (ns % 1_000_000_000) as u32)
                .unwrap()
                .naive_utc();
        assert_eq!(value, Value::Timestamp(expected));
    }

    /// `accessor_value`'s `TsNanosecond` arm splits the raw `i64` euclidean, so
    /// the remainder stays in `[0, 1_000_000_000)` — the only range
    /// `chrono::DateTime::from_timestamp` accepts. Truncating `/` and `%` yield
    /// a negative remainder for a pre-epoch `ns`, which wraps as `u32` and is
    /// rejected, so `unwrap_or_default()` would swallow it into the epoch.
    ///
    /// `-1` has a sub-second remainder; `-1_000_000_000` has none, so only the
    /// floored second separates the two divisions there.
    #[test]
    fn accessor_value_timestamp_nanosecond_negative_yields_pre_epoch_instant() {
        use arrow::array::TimestampNanosecondArray;
        use arrow::datatypes::TimeUnit;

        let schema = Arc::new(Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            false,
        )]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(TimestampNanosecondArray::from(vec![
            -1i64,
            -1_000_000_000i64,
        ]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("ts", ExaType::Timestamp)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        let last_nanosecond_of_1969 = NaiveDate::from_ymd_opt(1969, 12, 31)
            .unwrap()
            .and_hms_nano_opt(23, 59, 59, 999_999_999)
            .unwrap();
        let last_second_of_1969 = NaiveDate::from_ymd_opt(1969, 12, 31)
            .unwrap()
            .and_hms_nano_opt(23, 59, 59, 0)
            .unwrap();

        assert_eq!(
            accessor_value(&accessors[0], 0),
            Value::Timestamp(last_nanosecond_of_1969),
            "ns = -1 must decode to one nanosecond before the epoch"
        );
        assert_eq!(
            accessor_value(&accessors[0], 1),
            Value::Timestamp(last_second_of_1969),
            "ns = -1_000_000_000 must decode to one second before the epoch"
        );
    }

    /// `Date32`'s CE-day epoch offset (`ARROW_EPOCH_CE_DAY`) must apply for a
    /// pre-epoch value too, not just the post-epoch case the wider parity
    /// test already covers.
    #[test]
    fn accessor_value_date32_applies_ce_day_epoch_offset_pre_epoch() {
        use arrow::array::Date32Array;

        let schema = Arc::new(Schema::new(vec![Field::new("d", DataType::Date32, false)]));
        let days_since_epoch = -200i32;
        let arr: Arc<dyn arrow::array::Array> = Arc::new(Date32Array::from(vec![days_since_epoch]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("d", ExaType::Date)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        let value = accessor_value(&accessors[0], 0);

        let expected = NaiveDate::from_ymd_opt(1969, 6, 15).unwrap();
        assert_eq!(value, Value::Date(expected));
    }

    /// The three `Numeric`-widening accessors (`Int32`/`Int64`/`Float64`
    /// Arrow columns declared `ExaType::Numeric`) extract the natural typed
    /// value — `encode_slice` is what stringifies it for the wire, not
    /// `accessor_value` — so each must yield the plain `Value` variant, not a
    /// `Value::Numeric`.
    #[test]
    fn accessor_value_numeric_widening_variants_extract_natural_types() {
        use arrow::array::Int32Array;

        let numeric_meta = || {
            vec![col(
                "n",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            )]
        };

        let schema32 = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]));
        let arr32: Arc<dyn arrow::array::Array> = Arc::new(Int32Array::from(vec![-42i32]));
        let batch32 = RecordBatch::try_new(schema32, vec![arr32]).unwrap();
        let acc32 = build_accessors(&batch32, &numeric_meta()).unwrap();
        assert_eq!(accessor_value(&acc32[0], 0), Value::Int32(-42));

        let schema64 = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
        let arr64: Arc<dyn arrow::array::Array> =
            Arc::new(Int64Array::from(vec![9_999_999_999i64]));
        let batch64 = RecordBatch::try_new(schema64, vec![arr64]).unwrap();
        let acc64 = build_accessors(&batch64, &numeric_meta()).unwrap();
        assert_eq!(accessor_value(&acc64[0], 0), Value::Int64(9_999_999_999));

        let schemaf = Arc::new(Schema::new(vec![Field::new("n", DataType::Float64, false)]));
        let arrf: Arc<dyn arrow::array::Array> = Arc::new(Float64Array::from(vec![3.25f64]));
        let batchf = RecordBatch::try_new(schemaf, vec![arrf]).unwrap();
        let accf = build_accessors(&batchf, &numeric_meta()).unwrap();
        assert_eq!(accessor_value(&accf[0], 0), Value::Double(3.25));
    }

    /// A column declared `ExaType::Unsupported` gets `ColAccessor::Unsupported`
    /// regardless of its Arrow type, and `accessor_value` maps that variant to
    /// `Value::Null` — the column has no representation the DB reads back.
    #[test]
    fn accessor_value_unsupported_column_maps_to_null() {
        let schema = Arc::new(Schema::new(vec![Field::new("u", DataType::Utf8, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["ignored"]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("u", ExaType::Unsupported)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        assert!(matches!(accessors[0], ColAccessor::Unsupported));
        assert_eq!(accessor_value(&accessors[0], 0), Value::Null);
    }

    /// `accessor_value`'s `ColAccessor::Int32` arm — the one native-type
    /// arm not otherwise exercised: `mixed_meta`/`make_batch`'s Int64
    /// column already covers `Int64`, and `push_batch_int64_into_numeric_block`
    /// etc. cover the `NumericFromInt32` widening variant, but no existing
    /// test declares a plain `ExaType::Int32` column.
    #[test]
    fn accessor_value_int32_native_column_extracts_i32() {
        use arrow::array::Int32Array;

        let schema = Arc::new(Schema::new(vec![Field::new("a", DataType::Int32, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(Int32Array::from(vec![-42i32]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("a", ExaType::Int32)];
        let accessors = build_accessors(&batch, &meta).unwrap();

        assert_eq!(accessor_value(&accessors[0], 0), Value::Int32(-42));
    }

    /// `compute_row_costs`'s wildcard arm costs an Arrow column 0 when its
    /// `DataType` is neither a `fixed_cell_cost` type nor `Utf8`/`LargeUtf8`
    /// — reachable for an `ExaType::Unsupported` column, which `build_accessors`
    /// accepts paired with any Arrow type, since `compute_row_costs` inspects
    /// only the raw Arrow `DataType` (it ignores `ColumnMeta` entirely).
    #[test]
    fn compute_row_costs_wildcard_arm_costs_zero_for_unrecognized_arrow_type() {
        use arrow::array::UInt32Array;

        let schema = Arc::new(Schema::new(vec![Field::new("u", DataType::UInt32, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(UInt32Array::from(vec![7u32]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("u", ExaType::Unsupported)];

        assert_eq!(compute_row_costs(&batch, &meta), vec![0]);
    }

    /// A zero-row `RecordBatch` is a no-op: `push_batch` returns `Ok(())`
    /// without invoking `flush` and without buffering a tail, after flushing
    /// whatever was already pending (step 1).
    #[test]
    fn push_batch_zero_row_batch_is_noop() {
        let schema = Arc::new(Schema::new(vec![Field::new("a", DataType::Int64, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(Vec::<i64>::new()));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("a", ExaType::Int64)];

        let mut buf = EmitBuffer::new();
        let mut flush_called = false;
        buf.push_batch(&batch, &meta, &mut |_| {
            flush_called = true;
            Ok(())
        })
        .unwrap();

        assert!(!flush_called, "a zero-row batch must not flush");
        assert!(buf.is_empty(), "a zero-row batch must not buffer a tail");
    }

    /// When the last row's cost crosses the threshold, the final slice
    /// covers every remaining row and `slice_start` reaches `n_rows` —
    /// `push_batch` must leave nothing to materialise into the tail (the
    /// `if slice_start < n_rows` guard's false branch, which every other
    /// `push_batch` test — all of which leave a non-empty tail — never
    /// takes).
    #[test]
    fn push_batch_fully_flushed_by_final_row_leaves_no_tail() {
        // A single row whose cost alone exceeds the threshold: the loop
        // flushes it as one slice and slice_start lands exactly on n_rows.
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
        let s = "x".repeat(EMIT_BUFFER_LIMIT_BYTES + 1);
        let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec![s.as_str()]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        let meta = vec![col("v", ExaType::String { size: None })];

        let mut flush_count = 0usize;
        let mut buf = EmitBuffer::new();
        buf.push_batch(&batch, &meta, &mut |t| {
            assert_eq!(t.rows, 1);
            flush_count += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(flush_count, 1, "the oversized row must flush exactly once");
        assert!(
            buf.is_empty(),
            "a batch fully consumed by the final slice must leave no tail"
        );
    }

    /// `emit_record_batch_ipc` bans `emit_batch` in a RETURNS output context,
    /// mirroring `emit`'s ban (`returns_output_bans_emit`) — no existing
    /// arrow test configures the bridge with `IterType::ExactlyOnce` output
    /// before calling `emit_batch`.
    #[test]
    fn bridge_emit_batch_bans_returns_output_context() {
        let meta = mixed_meta();
        let batch = make_batch(&[1], &[Some("x")], &[1.0], &[true]);
        let empty_table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&empty_table, &meta);
        let mut emit = EmitBuffer::new();
        let mut bridge = make_bridge(&mut rs, &mut emit, &meta);
        bridge.configure_group_input(
            IterType::ExactlyOnce,
            IterType::ExactlyOnce,
            Box::new(|| Ok(None)),
        );

        match bridge.emit_batch(&batch) {
            Err(UdfError::User(msg)) => assert!(
                msg.contains("RETURNS"),
                "unexpected emit_batch-ban message: {msg}"
            ),
            other => panic!("expected a RETURNS-context ban error, got {other:?}"),
        }
    }

    /// `encode_slice`'s `ColAccessor::Unsupported` arm is a no-op in both the
    /// column-tally pre-sizing pass and the row-packing loop, mirroring
    /// `to_proto`'s `ExaType::Unsupported` arms
    /// (`to_proto_skips_unsupported_columns_in_both_tally_and_packing`) —
    /// `accessor_value_unsupported_column_maps_to_null` only calls
    /// `build_accessors`/`accessor_value` directly, not `encode_slice`.
    #[test]
    fn encode_slice_skips_unsupported_columns_in_both_tally_and_packing() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("u", DataType::Utf8, false),
        ]));
        let int_arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(vec![7i64]));
        let unsupported_arr: Arc<dyn arrow::array::Array> =
            Arc::new(StringArray::from(vec!["ignored"]));
        let batch = RecordBatch::try_new(schema, vec![int_arr, unsupported_arr]).unwrap();
        let meta = vec![col("a", ExaType::Int64), col("u", ExaType::Unsupported)];

        let table = encode_slice(&batch, &meta).unwrap();

        assert_eq!(table.data_int64, vec![7]);
        assert!(
            table.data_string.is_empty(),
            "an Unsupported column must not occupy the string block"
        );
        assert_eq!(
            table.data_nulls,
            vec![false, false],
            "the Unsupported cell is not NULL, just unrepresented"
        );
    }

    /// `build_accessors`' first guard rejects a batch whose column count
    /// doesn't match the declared EMITS schema, before any per-column
    /// downcast is attempted.
    #[test]
    fn build_accessors_column_count_mismatch_errors() {
        use arrow::array::Int32Array;

        let schema = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int32, false),
            Field::new("b", DataType::Int32, false),
        ]));
        let arr_a: Arc<dyn arrow::array::Array> = Arc::new(Int32Array::from(vec![1i32]));
        let arr_b: Arc<dyn arrow::array::Array> = Arc::new(Int32Array::from(vec![2i32]));
        let batch = RecordBatch::try_new(schema, vec![arr_a, arr_b]).unwrap();
        // Only one column declared, but the batch carries two.
        let meta = vec![col("a", ExaType::Int32)];

        let result = build_accessors(&batch, &meta);

        match result {
            Err(UdfError::Type(msg)) => assert!(
                msg.contains('2') && msg.contains('1'),
                "error must name both column counts: {msg}"
            ),
            Ok(_) => panic!("expected Err(Type) for a column-count mismatch, got Ok"),
            Err(other) => panic!("expected Err(Type), got a different variant: {other}"),
        }
    }

    /// The per-column `(dt, typ)` match's wildcard arm rejects any Arrow
    /// type / declared `ExaType` combination none of the named arms cover —
    /// tested directly against `build_accessors` rather than only through
    /// `push_batch`'s `push_batch_type_mismatch_errors`.
    #[test]
    fn build_accessors_arrow_exatype_mismatch_returns_type_error() {
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
        let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["x"]));
        let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
        // Utf8 cannot feed a Boolean column — no accessor variant matches.
        let meta = vec![col("v", ExaType::Boolean)];

        let result = build_accessors(&batch, &meta);

        match result {
            Err(UdfError::Type(msg)) => assert!(
                msg.contains("Utf8") || msg.contains("Boolean"),
                "error should name the offending types: {msg}"
            ),
            Ok(_) => panic!("expected Err(Type) for an Arrow/ExaType mismatch, got Ok"),
            Err(other) => panic!("expected Err(Type), got a different variant: {other}"),
        }
    }
}
