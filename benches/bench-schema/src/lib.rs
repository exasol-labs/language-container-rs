//! The benchmark suite's shared column schema, so `bench-udfs` (the UDF),
//! `exa-mock-db` (Tier 1's engine) and `udf-bench` (Tier 2's driver) agree on
//! the `wide` class without a dependency edge between them. No dependencies.

/// The wide class: `(name, SQL type, nullable)` per column. A third native, a
/// third string-block, a third VARCHAR of growing maximum length; every odd
/// column is nullable. Models a UDF that expands one input row (a file
/// reference) into millions of rows with dozens of columns.
pub const WIDE_COLUMNS: [(&str, &str, bool); 24] = [
    ("k", "DECIMAL(18,0)", false),
    ("i1", "DECIMAL(18,0)", true),
    ("i2", "DECIMAL(18,0)", false),
    ("d1", "DOUBLE", true),
    ("d2", "DOUBLE", false),
    ("d3", "DOUBLE", true),
    ("b1", "BOOLEAN", false),
    ("b2", "BOOLEAN", true),
    ("amt1", "DECIMAL(18,2)", false),
    ("amt2", "DECIMAL(18,2)", true),
    ("big1", "DECIMAL(36,10)", false),
    ("big2", "DECIMAL(36,10)", true),
    ("dt1", "DATE", false),
    ("dt2", "DATE", true),
    ("ts1", "TIMESTAMP", false),
    ("ts2", "TIMESTAMP", true),
    ("s8", "VARCHAR(8)", false),
    ("s16", "VARCHAR(16)", true),
    ("s32a", "VARCHAR(32)", false),
    ("s32b", "VARCHAR(32)", true),
    ("s64a", "VARCHAR(64)", false),
    ("s64b", "VARCHAR(64)", true),
    ("s128", "VARCHAR(128)", false),
    ("s200", "VARCHAR(200)", true),
];

/// Rows per Arrow record batch in the wide batch cells: the small and the
/// large end of what a file reader hands out.
pub const WIDE_BATCH_ROWS: [(&str, u64); 2] = [("batch8k", 8_192), ("batch64k", 65_536)];

/// [`WIDE_COLUMNS`] as a `name TYPE, ...` column list.
pub fn wide_ddl() -> String {
    WIDE_COLUMNS
        .iter()
        .map(|(name, ty, _)| format!("{name} {ty}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Declared size of a `VARCHAR(n)` type, `None` for any other type.
pub fn varchar_size(ty: &str) -> Option<u32> {
    ty.strip_prefix("VARCHAR(")?.strip_suffix(')')?.parse().ok()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
