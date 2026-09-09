# Feature: handshake-identity

Specifies the database-side semantics of the `exascript_info` identity fields (`current_user`, `scope_user`, `current_schema`, `script_schema`) that the handshake surfaces on `UdfMeta`, verified end to end against a live Exasol session.

## Background

The database fills the `exascript_info` identity fields from live session state, so their values are database behavior rather than host behavior. `current_user` names the user that executes the statement. `scope_user` equals `current_user` in every case except a UDF call wrapped in a view, where `scope_user` reports the view owner. The two fields therefore cannot diverge unless a view wraps the call.

Exasol documents no mapping from `IMPERSONATE` to `CURRENT_USER`. `EXA_DBA_SESSIONS` instead carries two user columns, `USER_NAME` for the logged-in user and `EFFECTIVE_USER` for the impersonated user. The IMPERSONATE scenario below records which of the two the `current_user` field reports.

`current_schema` names the schema the session has open, which `OPEN SCHEMA` and `CLOSE SCHEMA` change. `script_schema` names the schema that stores the script and stays fixed regardless of the open schema. Only a live-database test can verify these four fields, because the host copies them verbatim and cannot originate them. The scenarios below observe them end to end through the `current-user-meta` fixture, which renders an absent `Option` field as the literal `<none>`.

The wire-level accessor contract (`UdfContext` methods, `UdfMeta` shape, defaulting) is specified separately in `sdk/udf-sdk`; this feature specifies only what the database puts in those fields.

## Scenarios

### Scenario: Identity metadata reports the executing user and the open schema

* *GIVEN* a live Exasol session authenticated as user `SYS` with schema `IT_RUST` open, and a `RUST SCALAR SCRIPT` registered in `IT_RUST` that returns `current_user`, `scope_user`, `current_schema`, `script_schema`, and `script_name` as one pipe-delimited string
* *WHEN* the session selects that script
* *THEN* `current_user` MUST report `SYS`, the user that authenticated the session
* *AND* `scope_user` MUST report `SYS`, equal to `current_user`, because no view wraps the call
* *AND* `current_schema` and `script_schema` MUST both report `IT_RUST`, being the open schema and the script's home schema respectively
* *AND* none of `current_user`, `scope_user`, or `current_schema` MAY render as the fixture's `<none>` marker, because that marker denotes the accessor default rather than a live database value

### Scenario: current_schema tracks the open schema independently of script_schema

* *GIVEN* a live session with the script stored in schema `IT_RUST`, and a second schema `IT_RUST_OTHER`
* *WHEN* the session runs `OPEN SCHEMA IT_RUST_OTHER` and selects the script by its qualified name `IT_RUST.CURRENT_USER_META`
* *THEN* `current_schema` MUST report `IT_RUST_OTHER`, the schema the session has open
* *AND* `script_schema` MUST report `IT_RUST`, so the two fields MUST differ, proving `current_schema` follows `OPEN SCHEMA` while `script_schema` follows the script
* *AND* re-opening `IT_RUST` MUST return `current_schema` to `IT_RUST`, making the two fields equal again

### Scenario: A session with no open schema reports no current schema

* *GIVEN* a live session that has run `CLOSE SCHEMA`
* *WHEN* the session selects the script by its qualified name `IT_RUST.CURRENT_USER_META`
* *THEN* `current_schema` MUST NOT report `IT_RUST` or any other schema name
* *AND* the field MUST render as the literal string `NULL` — the database delivers the text `"NULL"` (not an SQL NULL) through the protocol when no schema is selected (this is ambiguous with a schema literally named `NULL`; tracked in [#93](https://github.com/exasol-labs/language-container-rs/issues/93))
* *AND* `script_schema` MUST still report `IT_RUST`, because the script's home schema is independent of session state
* *AND* re-opening `IT_RUST` MUST restore a reported `current_schema` of `IT_RUST`

### Scenario: scope_user reports the view owner when the script runs inside a view

* *GIVEN* a live session authenticated as `SYS` with the script stored in schema `IT_RUST`
* *AND* a second user `IT_VIEW_OWNER` that holds `USAGE ON SCHEMA IT_RUST` and `EXECUTE ON SCRIPT IT_RUST.CURRENT_USER_META`, because Exasol checks a view's underlying query against the view owner
* *AND* a view `IT_VIEW_SCOPE.V_CURRENT_USER_META` that selects the script, created inside schema `IT_VIEW_SCOPE` before `ALTER SCHEMA IT_VIEW_SCOPE CHANGE OWNER IT_VIEW_OWNER` re-owns the schema and every object it contains, making `IT_VIEW_OWNER` the view's owner
* *WHEN* the `SYS` session selects from that view
* *THEN* `current_user` MUST report `SYS`, the user that executes the statement
* *AND* `scope_user` MUST report `IT_VIEW_OWNER`, the owner of the view that wraps the script call, so the two fields MUST differ

### Scenario: IMPERSONATE establishes which user the current_user field reports

* *GIVEN* a live session authenticated as `SYS` that has already read the identity metadata once and recorded the reported `current_user`, and that holds no open write locks, because Exasol rejects `IMPERSONATE` while a transaction holds write locks
* *AND* a second database user `IT_IMPERSONATED` on which the session holds the `IMPERSONATION` privilege
* *WHEN* the session runs `IMPERSONATE IT_IMPERSONATED` and selects the same script again
* *THEN* the reported `current_user` MUST equal the value that `SELECT CURRENT_USER` returns in the same impersonated session, because one session MUST name one executing user
* *AND* the reported `current_user` MUST equal exactly one of `EXA_DBA_SESSIONS.USER_NAME` or `EXA_DBA_SESSIONS.EFFECTIVE_USER` read for that session, and the test MUST record which column matched, because Exasol documents no mapping from `IMPERSONATE` to `CURRENT_USER`
* *AND* `scope_user` MUST equal the reported `current_user` and `script_schema` MUST stay unchanged, because no view wraps the call and `IMPERSONATE` does not move the script
