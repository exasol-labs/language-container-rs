use exa_udf_runtime::InputRowSet;
use exa_zmq_protocol::{ColumnMeta, ExaType};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAllocator;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn col(name: &str, typ: ExaType) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        typ,
        type_name: String::new(),
        size: None,
        precision: None,
        scale: None,
    }
}

fn make_table(n_rows: usize) -> exa_proto::ExascriptTableData {
    exa_proto::ExascriptTableData {
        rows: n_rows as u64,
        rows_in_group: 0,
        data_int64: (0..n_rows as i64).collect(),
        data_string: (0..n_rows).map(|i| format!("row{i}")).collect(),
        data_nulls: vec![false; n_rows * 2],
        data_bool: vec![],
        data_int32: vec![],
        data_double: vec![],
        row_number: vec![],
    }
}

fn measure_allocs(n_rows: usize, meta: &[ColumnMeta]) -> usize {
    let table = make_table(n_rows);
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    let mut rs = InputRowSet::from_proto(table, meta);
    while rs.advance() {}
    ALLOC_COUNT.load(Ordering::SeqCst)
}

#[test]
fn from_proto_allocation_count_is_independent_of_row_count() {
    let meta = vec![
        col("a", ExaType::Int64),
        col("b", ExaType::String { size: None }),
    ];

    let _warmup = measure_allocs(10, &meta);

    let small = measure_allocs(100, &meta);
    let large = measure_allocs(10_000, &meta);

    assert_eq!(
        small, large,
        "allocation count must not grow with row count: {small} (100 rows) vs {large} (10000 rows)"
    );
}
