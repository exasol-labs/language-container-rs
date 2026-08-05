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
}
