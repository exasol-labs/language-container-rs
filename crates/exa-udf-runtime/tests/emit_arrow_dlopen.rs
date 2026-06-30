//! Boundary-safety gate (no DB): dlopen the host-built `libemit_arrow_batch.so`
//! and drive its `run`, which calls `ctx.emit_batch(&batch)` — now serialising
//! to Arrow IPC bytes UDF-side and crossing the boundary only as `&[u8]`. The
//! pre-IPC design SIGSEGV'd here; with IPC the `.so`-built batch must round-trip
//! cleanly into the host's emit buffer.
#![cfg(feature = "emit-arrow")]

use exa_proto::ExascriptTableData;
use exa_udf_runtime::{EmitBuffer, HandshakeMeta, HostContextBridge, InputRowSet, LoadedUdf};
use exa_zmq_protocol::{ColumnMeta, ExaType};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::value::Value;

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

#[test]
fn emit_arrow_batch_so_round_trips_via_ipc() {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../target/debug/libemit_arrow_batch.so");
    assert!(
        p.exists(),
        "build first: cargo build -p emit-arrow-batch ({p:?})"
    );

    let udf = LoadedUdf::open(&p, "EMIT_ARROW_BATCH").expect("load .so");

    let input_cols = vec![col("dummy", ExaType::Boolean)];
    let empty = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let mut input = InputRowSet::from_proto(&empty, &input_cols);
    let mut emit_buf = EmitBuffer::new();
    // EMITS (id BIGINT, label VARCHAR(1)) -> BIGINT arrives as Numeric.
    let output_meta = vec![
        col(
            "id",
            ExaType::Numeric {
                precision: None,
                scale: None,
            },
        ),
        col("label", ExaType::String { size: Some(1) }),
    ];

    let rc = {
        let mut bridge = HostContextBridge::new(
            &mut input,
            &mut emit_buf,
            &input_cols,
            &output_meta,
            Box::new(|_t: ExascriptTableData| Ok(())),
            HandshakeMeta::default(),
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        );
        let mut dyn_ref: &mut dyn UdfContext = &mut bridge;
        let ctx_ptr = &mut dyn_ref as *mut &mut dyn UdfContext as *mut std::ffi::c_void;
        let mut error_ptr: *mut std::ffi::c_char = std::ptr::null_mut();
        unsafe { udf.run(ctx_ptr, &mut error_ptr as *mut *mut std::ffi::c_char) }
    };
    assert_eq!(rc, 0, "UDF run returned non-zero");

    let table = emit_buf.to_proto(&output_meta);
    assert_eq!(table.rows, 3, "expected 3 emitted rows");
    let rs = InputRowSet::from_proto(&table, &output_meta);
    assert_eq!(rs.row(0).unwrap()[1], Value::String("a".into()));
    assert_eq!(rs.row(2).unwrap()[1], Value::String("c".into()));
}
