//! Boundary-safety gate (no DB): dlopen the fixture cdylibs and drive their `run`,
//! which emits a batch through the Arrow C Data Interface into the host's buffer.
// `emit-arrow-test`, not `emit-arrow`: the fixture cdylib is an optional
// dependency behind that feature. See `Cargo.toml`.
#![cfg(feature = "emit-arrow-test")]

use exa_proto::ExascriptTableData;
use exa_udf_runtime::{EmitBuffer, HandshakeMeta, HostContextBridge, InputRowSet, LoadedUdf};
use exa_zmq_protocol::{ColumnInfo, ExaType};
use exasol_udf_sdk::value::Value;

mod common;
use common::fixture_cdylib_path;

fn col(name: &str, typ: ExaType) -> ColumnInfo {
    ColumnInfo {
        name: name.to_string(),
        typ,
        type_name: String::new(),
        size: None,
        precision: None,
        scale: None,
    }
}

#[test]
fn emit_arrow_batch_so_round_trips_via_c_data_interface() {
    assert_emits_abc("emit_arrow_batch", "EMIT_ARROW_BATCH");
}

/// Scenario: emit_record_batch_ffi accepts a batch exported with the UDF's own arrow version
#[test]
fn emit_record_batch_ffi_accepts_older_arrow_major() {
    assert_emits_abc("emit_arrow_ffi", "EMIT_ARROW_FFI");
}

fn assert_emits_abc(lib: &str, entry: &str) {
    let p = fixture_cdylib_path(lib);
    let udf = LoadedUdf::open(&p, entry).expect("load .so");

    let input_cols = vec![col("dummy", ExaType::Boolean)];
    let empty = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut input = InputRowSet::from_proto(empty, &input_cols).unwrap();
    let mut emit_buf = EmitBuffer::new();
    // EMITS (id BIGINT, label VARCHAR(1)) -> BIGINT arrives as Numeric.
    let output_meta = vec![
        col(
            "id",
            ExaType::Numeric {
                precision: 18,
                scale: 0,
            },
        ),
        col("label", ExaType::String { size: 1 }),
    ];

    let result = {
        let mut bridge = HostContextBridge::new(
            &mut input,
            &mut emit_buf,
            &input_cols,
            &output_meta,
            Box::new(|_t: &exa_zmq_protocol::EmitTable| Ok(())),
            HandshakeMeta::default(),
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::Connection(
                    "no credential fetcher in test".into(),
                ))
            }),
        );
        udf.run(&mut bridge)
    };
    assert!(result.is_ok(), "UDF run failed: {result:?}");

    let table = emit_buf.take_proto();
    assert_eq!(table.rows, 3, "expected 3 emitted rows");
    let rs = InputRowSet::from_proto(table, &output_meta).unwrap();
    assert_eq!(rs.row(0).unwrap()[1], Value::String("a".into()));
    assert_eq!(rs.row(2).unwrap()[1], Value::String("c".into()));
}
