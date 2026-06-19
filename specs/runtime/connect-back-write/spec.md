# Feature: connect-back-write

Implements the host side of the connect-back DML/transaction surface inside the runtime: `RuntimeExaConnection` implements `begin`, `commit`, and `rollback` for explicit transaction control; `execute_batch` prepares a parameterised DML statement, executes it with all rows in one batch update call, and closes the prepared statement — the entire lifecycle driven on the dedicated `CONNECT_BACK_RT` tokio runtime and guarded by `catch_unwind`. A Value→Parameter mapping converts SDK values to `exarrow_rs::Parameter` variants before dispatch.

## Background

Connect-back write operations run on the dedicated `CONNECT_BACK_RT` tokio runtime so they can be called from synchronous UDF code. All async operations are wrapped in `block_on` and guarded by `catch_unwind` to prevent panics from unwinding across the FFI boundary. The transaction control methods (`begin`, `commit`, `rollback`) delegate directly to the underlying `exarrow_rs::Connection`. `execute_batch` follows a prepare-execute-close lifecycle, releasing the server-side prepared statement even on error.

## Scenarios

### Scenario: RuntimeExaConnection implements begin, commit, and rollback

* *GIVEN* a `Box<dyn ExaConnection>` returned by `ctx.connect_back` (a `RuntimeExaConnection` under the hood)
* *WHEN* the UDF calls `begin()`, `commit()`, or `rollback()` on the connection
* *THEN* each call MUST drive the corresponding `exarrow_rs::Connection` operation on the dedicated `CONNECT_BACK_RT` tokio runtime via `block_on`
* *AND* an `exarrow_rs::QueryError` from the operation MUST be mapped to `UdfError::ConnectBack(e.to_string())`
* *AND* a panic inside `block_on` MUST be caught by `catch_unwind` and returned as `UdfError::ConnectBack("panic in <op>: <payload>")` rather than unwinding across the FFI boundary

### Scenario: RuntimeExaConnection implements execute_batch via prepare-batch-close lifecycle

* *GIVEN* a `Box<dyn ExaConnection>` returned by `ctx.connect_back` (a `RuntimeExaConnection` under the hood)
* *WHEN* the UDF calls `execute_batch(sql, rows)` where `sql` is a parameterised DML statement (e.g. `DELETE … WHERE col1 = ? AND col2 = ?`) and `rows` is a non-empty slice of parameter rows
* *THEN* the runtime MUST call `Connection::prepare(sql)` on the dedicated `CONNECT_BACK_RT` tokio runtime to obtain a `PreparedStatement`
* *AND* MUST map each `Value` in every row to an `exarrow_rs::Parameter` via the Value→Parameter mapping (see "Value to Parameter mapping" scenario) to build a `Vec<Vec<Parameter>>`
* *AND* MUST call `Connection::execute_batch_update(&stmt, &param_rows)` on the same runtime, which sends the column-major batch to Exasol in one prepared-statement round-trip and returns the total affected-row count
* *AND* MUST call `Connection::close_prepared(stmt)` on the runtime to release server-side resources, even if `execute_batch_update` returned an error; if `close_prepared` itself fails its error MUST be logged but MUST NOT replace the original error
* *AND* MUST return the affected-row count as `u64` (clamping negative counts to 0) on success, or `UdfError::ConnectBack(e.to_string())` on failure
* *AND* any panic inside `block_on` or the async operations MUST be caught by `catch_unwind` and returned as `UdfError::ConnectBack("panic in execute_batch: <payload>")` rather than unwinding across the FFI boundary
* *AND* an empty `rows` slice MUST return `Ok(0)` immediately without opening a prepared statement on the server

### Scenario: Value to Parameter mapping for execute_batch

* *GIVEN* a `Vec<Vec<Value>>` parameter grid passed to `execute_batch`
* *WHEN* the runtime maps each `Value` to an `exarrow_rs::Parameter`
* *THEN* `Value::Null` MUST map to `Parameter::Null`
* *AND* `Value::Bool(b)` MUST map to `Parameter::Boolean(b)`
* *AND* `Value::Int32(i)` MUST map to `Parameter::Integer(i as i64)`
* *AND* `Value::Int64(i)` MUST map to `Parameter::Integer(i)`
* *AND* `Value::Double(f)` MUST map to `Parameter::Float(f)`
* *AND* `Value::String(s)` MUST map to `Parameter::String(s)`
* *AND* `Value::Numeric(_)`, `Value::Date(_)`, and `Value::Timestamp(_)` MUST each return `Err(UdfError::Unimplemented("execute_batch does not support Numeric/Date/Timestamp parameters — use String or Integer"))` rather than silently string-rendering, because a silently wrong literal is worse than a clear error; callers that need these types MUST convert them to `Value::String` before calling `execute_batch`
