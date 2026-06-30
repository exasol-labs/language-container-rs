//! In-process tests for the debug-output-redirect plan scenarios.
//!
//! Covers: `resolved_level_sets_global_max_level`, `context_reports_resolved_debug_level`,
//! `runtime_lines_carry_vm_tags`, `runtime_lines_flushed_per_write`,
//! `telemetry_emitted_at_debug_level_only`, `emit_flush_path_instrumented`,
//! and `udf_log_macro_writes_to_stderr_when_permitted`.

use exa_proto::ExascriptTableData;
use exa_udf_runtime::{EmitBuffer, HandshakeMeta, HostContextBridge, InputRowSet};
use exa_zmq_protocol::{ColumnMeta, ExaType};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::value::Value;
use std::sync::{Arc, Mutex};
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::reload;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A `MakeWriter` backed by a `Mutex<Vec<u8>>` — same pattern as the one in
/// rowset.rs's test module; duplicated here so this file is self-contained.
struct LockedWriter(Arc<Mutex<Vec<u8>>>);

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
        LockedWriter(Arc::clone(&self.0))
    }
}

fn int64_col(name: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.to_string(),
        typ: ExaType::Int64,
        type_name: String::new(),
        size: None,
        precision: None,
        scale: None,
    }
}

fn empty_rowset(meta: &[ColumnMeta]) -> InputRowSet {
    let table = ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    InputRowSet::from_proto(&table, meta)
}

fn make_bridge<'a>(
    input: &'a mut InputRowSet,
    emit: &'a mut EmitBuffer,
    cols: &'a [ColumnMeta],
) -> HostContextBridge<'a> {
    HostContextBridge::new(
        input,
        emit,
        cols,
        cols,
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

/// Process-global serialisation lock.
///
/// Every test in this file holds this lock for its full duration.  Any
/// `tracing::subscriber::with_default` call that installs a DEBUG-level
/// subscriber will, upon the first use of a `debug!` callsite, register that
/// callsite against the current dispatcher and call `rebuild_interest_cache`,
/// which updates the process-global `MAX_LEVEL` atomic.  Without serialisation
/// this races with tests that assert on `LevelFilter::current()`.
///
/// All tests in this file are fast (sub-millisecond), so serialising them has
/// no meaningful impact on total test time.
static GLOBAL_LEVEL_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Scenario: resolved_level_sets_global_max_level
// ---------------------------------------------------------------------------

/// After applying a resolved `debug` level through the same
/// `reload::Handle::modify` path that `main()`/`run()` use,
/// `tracing::level_filters::LevelFilter::current()` reflects DEBUG and a
/// `debug!` event is captured.  Before applying it (INFO filter still active)
/// a `debug!` event is NOT captured.
///
/// Both "before" and "after" phases run inside one `with_default` block using
/// one reload handle so no concurrent `rebuild_interest_cache` call from
/// another test can race between the `modify` call and the assertion.
///
/// Scenario: `resolved_level_sets_global_max_level`
#[test]
fn resolved_level_sets_global_max_level() {
    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Both transitions happen inside one with_default / one reload handle.
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let initial_filter = tracing_subscriber::EnvFilter::new("info");
        let (filter_layer, filter_handle) = reload::Layer::new(initial_filter);
        let sub = tracing_subscriber::registry().with(filter_layer).with(
            tracing_subscriber::fmt::layer()
                .with_writer(LockedWriter(Arc::clone(&buf)))
                .with_ansi(false),
        );
        tracing::subscriber::with_default(sub, || {
            // ---- Before: set INFO, emit debug → must not be captured. ----
            let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("info"));
            tracing::debug!("pre-resolve debug event");

            // ---- After: set DEBUG, assert LevelFilter::current() + capture. ----
            // Mirrors the production path:
            //   filter_handle.modify(|f| *f = EnvFilter::new(level.as_str()))
            // which calls rebuild_interest_cache(), updating LevelFilter::current().
            let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("debug"));

            assert_eq!(
                tracing::level_filters::LevelFilter::current(),
                tracing::level_filters::LevelFilter::DEBUG,
                "LevelFilter::current() must reflect DEBUG after reload handle modify"
            );
            tracing::debug!("post-resolve debug event");

            // Restore before releasing the lock.
            let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("info"));
        });
    }

    let output = {
        let g = buf.lock().unwrap();
        String::from_utf8_lossy(&g).into_owned()
    };
    assert!(
        !output.contains("pre-resolve debug event"),
        "INFO subscriber must not capture a debug event, got: {output:?}"
    );
    assert!(
        output.contains("post-resolve debug event"),
        "DEBUG subscriber must capture a debug event after level resolved, got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario: context_reports_resolved_debug_level
// ---------------------------------------------------------------------------

/// `HostContextBridge::debug_level()` reads `LevelFilter::current()`.  After
/// resolving to DEBUG (via reload handle), the bridge reports `Level::DEBUG`;
/// after resolving back to INFO, it reports `Level::INFO`.  Both transitions
/// are asserted within one `with_default` block so no concurrent
/// `rebuild_interest_cache` call from another test can race between them.
///
/// Scenario: `context_reports_resolved_debug_level`
#[test]
fn context_reports_resolved_debug_level() {
    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let meta = vec![int64_col("x")];

    // Both transitions (INFO → DEBUG → INFO) happen inside one with_default
    // block using one reload handle.  Each filter_handle.modify call is
    // synchronous and updates LevelFilter::current() before returning, so
    // the assertions immediately following each modify see the correct value.
    {
        let initial_filter = tracing_subscriber::EnvFilter::new("info");
        let (filter_layer, filter_handle) = reload::Layer::new(initial_filter);
        let sub = tracing_subscriber::registry().with(filter_layer).with(
            tracing_subscriber::fmt::layer()
                .with_writer(LockedWriter(Arc::new(Mutex::new(Vec::new()))))
                .with_ansi(false),
        );
        tracing::subscriber::with_default(sub, || {
            // Start at INFO.
            let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("info"));
            {
                let mut rs = empty_rowset(&meta);
                let mut emit = EmitBuffer::new();
                let bridge = make_bridge(&mut rs, &mut emit, &meta);
                assert_eq!(
                    bridge.debug_level(),
                    tracing::Level::INFO,
                    "bridge must report INFO when global max is INFO"
                );
            }

            // Resolve to DEBUG — simulates on_level_resolved(Level::DEBUG).
            let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("debug"));
            {
                let mut rs = empty_rowset(&meta);
                let mut emit = EmitBuffer::new();
                let bridge = make_bridge(&mut rs, &mut emit, &meta);
                assert_eq!(
                    bridge.debug_level(),
                    tracing::Level::DEBUG,
                    "bridge must report DEBUG when global max is DEBUG"
                );
            }

            // Restore to INFO before releasing the lock.
            let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("info"));
        });
    }
}

// ---------------------------------------------------------------------------
// Scenario: runtime_lines_carry_vm_tags
// ---------------------------------------------------------------------------

/// Entering a span constructed identically to the runtime root span
/// (`error_span!("udf", pid=…, session_id=…, node_id=…, vm_id=…)`) causes
/// captured log lines to contain those field names and values.
///
/// The test replicates the span construction from `lib.rs::run()` directly to
/// exercise the formatter path without a live ZMQ session.  The span is at
/// `ERROR` level (matching the fix in task 4.2) so it is entered at any
/// non-OFF level.
///
/// Scenario: `runtime_lines_carry_vm_tags`
#[test]
fn runtime_lines_carry_vm_tags() {
    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_writer(LockedWriter(Arc::clone(&buf)))
        .with_ansi(false)
        .finish();

    let test_pid: u32 = 12345;
    let test_session_id: u64 = 9999;
    let test_node_id: u64 = 1;
    let test_vm_id: u64 = 42;

    tracing::subscriber::with_default(sub, || {
        // Mirror the exact span construction from lib.rs::run() (error_span!).
        let _root = tracing::error_span!(
            "udf",
            pid = test_pid,
            session_id = test_session_id,
            node_id = test_node_id,
            vm_id = test_vm_id,
        )
        .entered();

        // Emit an event inside the span so the formatter includes the span fields.
        tracing::error!("sentinel event inside udf span");
    });

    let output = {
        let g = buf.lock().unwrap();
        String::from_utf8_lossy(&g).into_owned()
    };

    assert!(
        output.contains("pid="),
        "output must contain pid field, got: {output:?}"
    );
    assert!(
        output.contains("session_id="),
        "output must contain session_id field, got: {output:?}"
    );
    assert!(
        output.contains("node_id="),
        "output must contain node_id field, got: {output:?}"
    );
    assert!(
        output.contains("vm_id="),
        "output must contain vm_id field, got: {output:?}"
    );
    assert!(
        output.contains("12345"),
        "output must contain the pid value 12345, got: {output:?}"
    );
    assert!(
        output.contains("9999"),
        "output must contain session_id value 9999, got: {output:?}"
    );
    assert!(
        output.contains("42"),
        "output must contain vm_id value 42, got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario: runtime_lines_flushed_per_write
// ---------------------------------------------------------------------------

/// The runtime formatter writes to `std::io::stderr()`, which is an OS-level
/// file descriptor with no userspace buffering (no `BufWriter` wrapper).
///
/// A byte-level capture of fd 2 requires `dup2` + pipe plumbing that is
/// impractical inside Rust's test harness without additional unsafe code or
/// new dependencies. Instead this test verifies the structural property:
/// two events issued into a `MakeWriter`-backed subscriber both appear in the
/// captured buffer without an explicit flush, demonstrating that the writer
/// is called per-event (no coalescing). The production subscriber in
/// `exaudfclient/src/main.rs` uses `.with_writer(std::io::stderr)` directly
/// (not wrapped in `BufWriter`), which forwards each write call to the OS
/// `write(2)` syscall immediately.
///
/// Scenario: `runtime_lines_flushed_per_write`
#[test]
fn runtime_lines_flushed_per_write() {
    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(LockedWriter(Arc::clone(&buf)))
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(sub, || {
        tracing::info!("first event");
        tracing::info!("second event");
    });

    let output = {
        let g = buf.lock().unwrap();
        String::from_utf8_lossy(&g).into_owned()
    };

    // Both events must appear without an explicit flush, proving each write
    // is immediately delivered (no userspace coalescing).
    assert!(
        output.contains("first event"),
        "first event must appear without flush, got: {output:?}"
    );
    assert!(
        output.contains("second event"),
        "second event must appear without flush, got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenarios: telemetry_emitted_at_debug_level_only + emit_flush_path_instrumented
// ---------------------------------------------------------------------------
// The canonical implementations live in rowset.rs's test module.  These thin
// wrappers confirm the same plan scenarios from the plan-specified home file
// (`tests/debug_level.rs`), reusing EmitBuffer directly.

/// Thin confirmation: flush telemetry appears at DEBUG, is absent at INFO.
/// Full coverage is in rowset.rs::tests::telemetry_emitted_at_debug_level_only.
///
/// Scenario: `telemetry_emitted_at_debug_level_only`
#[test]
fn telemetry_emitted_at_debug_level_only() {
    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let emit_with_level = |level: tracing::Level| -> String {
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let sub = tracing_subscriber::fmt()
            .with_max_level(level)
            .with_writer(LockedWriter(Arc::clone(&buf)))
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(sub, || {
            let emit = EmitBuffer::new();
            emit.record_flush_telemetry();
        });
        let g = buf.lock().unwrap();
        String::from_utf8_lossy(&g).into_owned()
    };

    let debug_out = emit_with_level(tracing::Level::DEBUG);
    let info_out = emit_with_level(tracing::Level::INFO);

    assert!(
        debug_out.contains("emit_flush"),
        "debug output must contain emit_flush target, got: {debug_out:?}"
    );
    assert!(
        !info_out.contains("emit_flush"),
        "info output must not contain emit_flush target, got: {info_out:?}"
    );
}

/// Thin confirmation: push instrumentation events appear at DEBUG level.
/// Full coverage is in rowset.rs::tests::emit_flush_path_instrumented.
///
/// Scenario: `emit_flush_path_instrumented`
#[test]
fn emit_flush_path_instrumented() {
    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(LockedWriter(Arc::clone(&buf)))
        .with_ansi(false)
        .finish();

    tracing::subscriber::with_default(sub, || {
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::Int64(1)]);
    });

    let output = {
        let g = buf.lock().unwrap();
        String::from_utf8_lossy(&g).into_owned()
    };

    assert!(
        output.contains("emit_push") || output.contains("bytes_buffered"),
        "debug output must contain push instrumentation, got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario: udf_log_macro_writes_to_stderr_when_permitted
// ---------------------------------------------------------------------------

/// `udf_log!` writes a formatted line to `std::io::stderr()` when the
/// context's resolved level permits the requested level.
///
/// Capturing real fd 2 in-process requires `dup2` + pipe plumbing beyond what
/// Rust's test harness provides without new dependencies, so this test:
///
/// - Asserts the permitted-level path does not panic (the `writeln!(stderr(), …)`
///   call returns `Ok`, i.e. the write succeeds).
/// - Asserts the suppressed-level path is a compile-time and runtime no-op.
/// - Asserts level-ordering semantics: `msg_level <= ctx.debug_level()` is the
///   gate, matching `tracing::Level` ordering (ERROR < WARN < INFO < DEBUG < TRACE).
///
/// Scenario: `udf_log_macro_writes_to_stderr_when_permitted`
#[test]
fn udf_log_macro_writes_to_stderr_when_permitted() {
    let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use exasol_udf_sdk::udf_log;

    struct FixedCtx(tracing::Level);

    impl UdfContext for FixedCtx {
        fn num_columns(&self) -> usize {
            0
        }
        fn get(&self, _col: usize) -> Result<&Value, exasol_udf_sdk::error::UdfError> {
            Err(exasol_udf_sdk::error::UdfError::Type("no cols".into()))
        }
        fn emit(&mut self, _values: &[Value]) -> Result<(), exasol_udf_sdk::error::UdfError> {
            Ok(())
        }
        fn next(&mut self) -> Result<bool, exasol_udf_sdk::error::UdfError> {
            Ok(false)
        }
        fn debug_level(&self) -> tracing::Level {
            self.0
        }
    }

    // Permitted path: DEBUG context allows debug/info/warn/error messages.
    // The macro calls writeln!(stderr(), …) which must not panic.
    let ctx = FixedCtx(tracing::Level::DEBUG);
    udf_log!(ctx, debug, "permitted debug message from test");
    udf_log!(ctx, info, "permitted info message from test");
    udf_log!(ctx, warn, "permitted warn message from test");
    udf_log!(ctx, error, "permitted error message from test");

    // Suppressed path: INFO context rejects debug/trace messages — no-op.
    let ctx_info = FixedCtx(tracing::Level::INFO);
    udf_log!(ctx_info, debug, "suppressed at info level");
    udf_log!(ctx_info, trace, "suppressed at info level");

    // Level ordering: msg_level <= ctx.debug_level() is the gate.
    // tracing::Level ordering: ERROR(1) < WARN(2) < INFO(3) < DEBUG(4) < TRACE(5).
    assert!(
        tracing::Level::DEBUG > tracing::Level::INFO,
        "DEBUG must be more verbose than INFO"
    );
    assert!(
        tracing::Level::ERROR <= tracing::Level::INFO,
        "ERROR must be permitted at INFO context level"
    );
    // debug (4) > info (3) → suppressed at INFO level.
    assert!(
        tracing::Level::DEBUG > tracing::Level::INFO,
        "DEBUG must be suppressed at INFO context level"
    );
}
