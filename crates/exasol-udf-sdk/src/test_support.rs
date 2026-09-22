//! `UdfContext` doubles for unit-testing a UDF without a live host.
//!
//! A UDF function takes `&mut dyn UdfContext`, so testing one off-database
//! needs an implementation of that trait. Hand-writing it per crate produced 22
//! near-identical blocks in this repository behind only 7 distinct behaviors,
//! and every new required trait method cost 22 edits. This module owns that
//! double once, behind the non-default `test-support` feature.
//!
//! Two doubles ship, not one, because they answer opposite questions.
//! [`TestContext`] is the working double: it overrides the provided
//! `UdfContext` methods so a test can supply input rows, inspect emitted rows,
//! read back the RETURNS value, and set handshake metadata. That override is
//! exactly the hazard for the other kind of test. A test asserting what a
//! *provided* method does by default (`memory_limit()` returning `0`,
//! `set_return` reporting `Unimplemented`) would, given a `TestContext`,
//! observe the double's re-implementation and pass no matter what the trait
//! default became. Rust offers no way to opt out of an override per test, so
//! [`DefaultsCtx`] exists as a double that overrides nothing beyond the four
//! required methods and therefore cannot shadow a default under test.

use std::collections::HashMap;

use crate::connect_back::ConnectionObject;
use crate::context::{InputType, OutputType, UdfContext};
use crate::error::UdfError;
use crate::value::{ColumnInfo, Value};

/// What [`TestContext::emit`] does with a row.
#[derive(Debug, Default)]
pub enum EmitPolicy {
    /// Accept the row and append it to [`TestContext::emitted`].
    #[default]
    Record,
    /// Reject every call with a copy of this error and record nothing, so a
    /// test can drive the host's ban on `emit` in RETURNS output.
    Reject(UdfError),
}

/// What [`TestContext::next`] does at a call.
#[derive(Debug, Default)]
pub enum NextPolicy {
    /// Advance the cursor over the rows the constructor supplied.
    #[default]
    Advance,
    /// Reject every call with a copy of this error, so a test can drive the
    /// host's ban on `next` in scalar input.
    Reject(UdfError),
}

/// A `UdfContext` over caller-supplied rows, for unit tests with no host.
///
/// Construct it with [`TestContext::scalar`] for a scalar (one row per
/// invocation) UDF or [`TestContext::set`] for a SET group, then read output
/// back with [`TestContext::emitted`] and [`TestContext::captured_return`].
/// `input_column_count` follows the supplied row data, so no separate column count can
/// drift out of step with the rows.
///
/// Two policies replace a knob per behavior: [`EmitPolicy`] and [`NextPolicy`]
/// each turn their method into a caller-supplied failure, which is how a test
/// reproduces a host that bans the call. The out-of-range `get` error is
/// deliberately not configurable: it is always [`UdfError::Type`], because a
/// caller that could choose the kind would be choosing a detail no test
/// asserts.
///
/// Prefer [`DefaultsCtx`] in a test that asserts a trait default. `TestContext`
/// overrides the provided methods and would shadow the default under test.
#[derive(Debug)]
pub struct TestContext {
    rows: Vec<Vec<Value>>,
    /// 1-based index of the current row; `0` means the cursor sits before the
    /// first row, which is where a SET group starts.
    cursor: usize,
    emitted: Vec<Vec<Value>>,
    captured_return: Option<Option<Value>>,
    emit_policy: EmitPolicy,
    next_policy: NextPolicy,
    meta: Metadata,
    input_columns: Vec<ColumnInfo>,
    output_columns: Vec<ColumnInfo>,
}

impl TestContext {
    /// A scalar-input context over one row, already positioned on it.
    ///
    /// `next` reports exhaustion, matching the host's scalar contract where the
    /// framework drives one invocation per row.
    pub fn scalar(row: Vec<Value>) -> Self {
        Self::positioned(vec![row], 1)
    }

    /// A SET-input context over a whole group, positioned before the first row.
    ///
    /// The caller must call `next` before the first `get`, as against a live
    /// host. A `get` before that returns an error rather than panicking.
    pub fn set(rows: Vec<Vec<Value>>) -> Self {
        Self::positioned(rows, 0)
    }

    fn positioned(rows: Vec<Vec<Value>>, cursor: usize) -> Self {
        Self {
            rows,
            cursor,
            emitted: Vec::new(),
            captured_return: None,
            emit_policy: EmitPolicy::default(),
            next_policy: NextPolicy::default(),
            meta: Metadata::default(),
            input_columns: Vec::new(),
            output_columns: Vec::new(),
        }
    }

    /// Rows passed to `emit`, in call order.
    pub fn emitted(&self) -> &[Vec<Value>] {
        &self.emitted
    }

    /// The value passed to `set_return`, or `None` when it was never called.
    ///
    /// The nesting is load-bearing: `None` means the UDF never returned a
    /// value, `Some(&None)` means it returned SQL NULL.
    pub fn captured_return(&self) -> Option<&Option<Value>> {
        self.captured_return.as_ref()
    }

    /// Set the input column metadata this context reports. Unset by default, so
    /// `input_column` errors unless a test supplies a schema.
    pub fn with_input_columns(mut self, columns: Vec<ColumnInfo>) -> Self {
        self.input_columns = columns;
        self
    }

    /// Set the output column metadata this context reports.
    pub fn with_output_columns(mut self, columns: Vec<ColumnInfo>) -> Self {
        self.output_columns = columns;
        self
    }

    /// Replace what `emit` does with a row.
    pub fn with_emit_policy(mut self, policy: EmitPolicy) -> Self {
        self.emit_policy = policy;
        self
    }

    /// Replace what `next` does at a call.
    pub fn with_next_policy(mut self, policy: NextPolicy) -> Self {
        self.next_policy = policy;
        self
    }

    /// Set the sandbox memory limit this context reports.
    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.meta.memory_limit = bytes;
        self
    }

    /// Set the session ID this context reports.
    pub fn with_session_id(mut self, id: u64) -> Self {
        self.meta.session_id = id;
        self
    }

    /// Set the statement number this context reports.
    pub fn with_statement_id(mut self, id: u32) -> Self {
        self.meta.statement_id = id;
        self
    }

    /// Set the cluster node ID this context reports.
    pub fn with_node_id(mut self, id: u32) -> Self {
        self.meta.node_id = id;
        self
    }

    /// Set the cluster node count this context reports.
    pub fn with_node_count(mut self, count: u32) -> Self {
        self.meta.node_count = count;
        self
    }

    /// Set the VM ID this context reports.
    pub fn with_vm_id(mut self, id: u64) -> Self {
        self.meta.vm_id = id;
        self
    }

    /// Set the database name this context reports.
    pub fn with_database_name(mut self, name: impl Into<String>) -> Self {
        self.meta.database_name = name.into();
        self
    }

    /// Set the database version this context reports.
    pub fn with_database_version(mut self, version: impl Into<String>) -> Self {
        self.meta.database_version = version.into();
        self
    }

    /// Set the script name this context reports.
    pub fn with_script_name(mut self, name: impl Into<String>) -> Self {
        self.meta.script_name = name.into();
        self
    }

    /// Set the script schema this context reports.
    pub fn with_script_schema(mut self, schema: impl Into<String>) -> Self {
        self.meta.script_schema = schema.into();
        self
    }

    /// Set the current user this context reports. Absent by default, as when
    /// the database omits the optional handshake field.
    pub fn with_current_user(mut self, user: impl Into<String>) -> Self {
        self.meta.current_user = Some(user.into());
        self
    }

    /// Set the current schema this context reports. Absent by default, as when
    /// the database omits the optional handshake field.
    pub fn with_current_schema(mut self, schema: impl Into<String>) -> Self {
        self.meta.current_schema = Some(schema.into());
        self
    }

    /// Set the scope user this context reports. Absent by default, as when the
    /// database omits the optional handshake field.
    pub fn with_scope_user(mut self, user: impl Into<String>) -> Self {
        self.meta.scope_user = Some(user.into());
        self
    }

    /// Set the resolved verbosity this context reports to `udf_log!`.
    pub fn with_debug_level(mut self, level: tracing::Level) -> Self {
        self.meta.debug_level = level;
        self
    }

    /// Set the input-batch row count this context reports.
    pub fn with_rows_in_group(mut self, rows: u64) -> Self {
        self.meta.rows_in_group = rows;
        self
    }

    /// Set the declared input iteration axis this context reports.
    pub fn with_input_type(mut self, input_type: InputType) -> Self {
        self.meta.input_type = Some(input_type);
        self
    }

    /// Set the declared output iteration axis this context reports.
    pub fn with_output_type(mut self, output_type: OutputType) -> Self {
        self.meta.output_type = Some(output_type);
        self
    }

    /// Set the cluster IP this context reports.
    pub fn with_cluster_ip(mut self, ip: impl Into<String>) -> Self {
        self.meta.cluster_ip = Some(ip.into());
        self
    }

    /// Register a named CONNECTION object this context can look up.
    pub fn with_connection(mut self, name: impl Into<String>, obj: ConnectionObject) -> Self {
        self.meta
            .connections
            .insert(name.into().to_ascii_uppercase(), obj);
        self
    }

    fn current_row(&self) -> Option<&Vec<Value>> {
        self.cursor.checked_sub(1).and_then(|i| self.rows.get(i))
    }
}

impl UdfContext for TestContext {
    fn input_column_count(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
    }

    fn input_column(&self, idx: usize) -> Result<&ColumnInfo, UdfError> {
        self.input_columns
            .get(idx)
            .ok_or_else(|| UdfError::Type(format!("input column {idx} out of range")))
    }

    fn output_column_count(&self) -> usize {
        self.output_columns.len()
    }

    fn output_column(&self, idx: usize) -> Result<&ColumnInfo, UdfError> {
        self.output_columns
            .get(idx)
            .ok_or_else(|| UdfError::Type(format!("output column {idx} out of range")))
    }

    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        let Some(row) = self.current_row() else {
            return Err(UdfError::Type(format!(
                "get({col}) with no current row; call next() first"
            )));
        };
        row.get(col).ok_or_else(|| {
            UdfError::Type(format!(
                "get({col}) out of range; the current row has {} columns",
                row.len()
            ))
        })
    }

    fn emit(&mut self, values: Vec<Value>) -> Result<(), UdfError> {
        match &self.emit_policy {
            EmitPolicy::Record => {
                self.emitted.push(values);
                Ok(())
            }
            EmitPolicy::Reject(error) => Err(error.clone()),
        }
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        if let NextPolicy::Reject(error) = &self.next_policy {
            return Err(error.clone());
        }
        if self.cursor < self.rows.len() {
            self.cursor += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn set_return(&mut self, value: Option<Value>) -> Result<(), UdfError> {
        self.captured_return = Some(value);
        Ok(())
    }

    fn memory_limit(&self) -> u64 {
        self.meta.memory_limit
    }

    fn session_id(&self) -> u64 {
        self.meta.session_id
    }

    fn statement_id(&self) -> u32 {
        self.meta.statement_id
    }

    fn node_id(&self) -> u32 {
        self.meta.node_id
    }

    fn node_count(&self) -> u32 {
        self.meta.node_count
    }

    fn vm_id(&self) -> u64 {
        self.meta.vm_id
    }

    fn database_name(&self) -> String {
        self.meta.database_name.clone()
    }

    fn database_version(&self) -> String {
        self.meta.database_version.clone()
    }

    fn script_name(&self) -> String {
        self.meta.script_name.clone()
    }

    fn script_schema(&self) -> String {
        self.meta.script_schema.clone()
    }

    fn current_user(&self) -> Option<String> {
        self.meta.current_user.clone()
    }

    fn current_schema(&self) -> Option<String> {
        self.meta.current_schema.clone()
    }

    fn scope_user(&self) -> Option<String> {
        self.meta.scope_user.clone()
    }

    fn debug_level(&self) -> tracing::Level {
        self.meta.debug_level
    }

    fn rows_in_group(&self) -> u64 {
        self.meta.rows_in_group
    }

    fn input_type(&self) -> Option<InputType> {
        self.meta.input_type
    }

    fn output_type(&self) -> Option<OutputType> {
        self.meta.output_type
    }

    fn cluster_ip(&self) -> Result<String, UdfError> {
        self.meta
            .cluster_ip
            .clone()
            .ok_or_else(|| UdfError::Unimplemented("cluster_ip not set on TestContext".into()))
    }

    fn connection(&self, name: &str) -> Result<ConnectionObject, UdfError> {
        self.meta
            .connections
            .get(&name.to_ascii_uppercase())
            .cloned()
            .ok_or_else(|| {
                UdfError::Unimplemented(format!("connection {name:?} not set on TestContext"))
            })
    }
}

/// A `UdfContext` that overrides only the four required methods.
///
/// Use it wherever a test asserts what a *provided* `UdfContext` method does on
/// its own, such as `memory_limit()` returning `0` or `set_return` reporting
/// `Unimplemented`. [`TestContext`] overrides those methods, so the same
/// assertion made against it would verify the double instead of the trait and
/// keep passing after the trait default changed. `DefaultsCtx` cannot shadow a
/// default because it declares no override to shadow with.
///
/// It reports zero input columns, errors from `get`, accepts `emit` as a no-op,
/// and reports exhaustion from `next`, which is all a test of a provided method
/// needs from the required four.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultsCtx;

impl UdfContext for DefaultsCtx {
    fn input_column_count(&self) -> usize {
        0
    }

    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Type(format!(
            "get({col}) out of range; this context has no input columns"
        )))
    }

    fn emit(&mut self, _values: Vec<Value>) -> Result<(), UdfError> {
        Ok(())
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }
}

#[derive(Debug)]
struct Metadata {
    memory_limit: u64,
    session_id: u64,
    statement_id: u32,
    node_id: u32,
    node_count: u32,
    vm_id: u64,
    database_name: String,
    database_version: String,
    script_name: String,
    script_schema: String,
    current_user: Option<String>,
    current_schema: Option<String>,
    scope_user: Option<String>,
    debug_level: tracing::Level,
    rows_in_group: u64,
    input_type: Option<InputType>,
    output_type: Option<OutputType>,
    cluster_ip: Option<String>,
    connections: HashMap<String, ConnectionObject>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            memory_limit: 0,
            session_id: 0,
            statement_id: 0,
            node_id: 0,
            node_count: 0,
            vm_id: 0,
            database_name: String::new(),
            database_version: String::new(),
            script_name: String::new(),
            script_schema: String::new(),
            current_user: None,
            current_schema: None,
            scope_user: None,
            debug_level: tracing::Level::INFO,
            rows_in_group: 0,
            input_type: None,
            output_type: None,
            cluster_ip: None,
            connections: HashMap::new(),
        }
    }
}

#[cfg(test)]
#[path = "test_support_tests.rs"]
mod tests;
