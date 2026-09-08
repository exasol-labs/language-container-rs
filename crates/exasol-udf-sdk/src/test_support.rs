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

use crate::context::UdfContext;
use crate::error::UdfError;
use crate::value::Value;

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
/// `num_columns` follows the supplied row data, so no separate column count can
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

    fn current_row(&self) -> Option<&Vec<Value>> {
        self.cursor.checked_sub(1).and_then(|i| self.rows.get(i))
    }
}

impl UdfContext for TestContext {
    fn num_columns(&self) -> usize {
        self.rows.first().map_or(0, Vec::len)
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

    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
        match &self.emit_policy {
            EmitPolicy::Record => {
                self.emitted.push(values.to_vec());
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
    fn num_columns(&self) -> usize {
        0
    }

    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Type(format!(
            "get({col}) out of range; this context has no input columns"
        )))
    }

    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
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
        }
    }
}

#[cfg(test)]
#[path = "test_support_tests.rs"]
mod tests;
