use crate::error::UdfError;
use arrow::record_batch::RecordBatch;

/// Options for opening a connect-back connection.
pub enum ConnectBackOptions {
    /// Use the credentials surfaced during the handshake (default).
    Default,
    /// Override with a named connection object from the database.
    Named(String),
    /// Fully explicit credentials.
    Explicit {
        dsn: String,
        user: String,
        password: String,
    },
}

/// A live Exasol connection the UDF can use for queries and DML.
///
/// The trait is object-safe so the runtime can hand back a
/// `Box<dyn ExaConnection>`; the `Send` bound lets that box move across the
/// call boundaries the runtime manages.
pub trait ExaConnection: Send {
    /// Run a query and collect the result as Arrow record batches.
    fn query_arrow(&mut self, sql: &str) -> Result<Vec<RecordBatch>, UdfError>;
    /// Execute a DML/DDL statement, returning the affected row count.
    fn execute(&mut self, sql: &str) -> Result<u64, UdfError>;
}
