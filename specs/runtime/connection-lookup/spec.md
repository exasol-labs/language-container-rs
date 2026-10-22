# Feature: connection-lookup

`ctx.connection(name)` returns the credentials of a named database `CONNECTION` object as a `ConnectionObject`. The lookup is an `MT_IMPORT` request on the UDF's own control channel, not connect-back: it opens no SQL session and works without the runtime's `connect-back` feature. `HostContextBridge` and `SingleCallContext` serve it; `CleanupContext` refuses it (see `runtime/dispatch-cleanup-hook`).

## Background

The database answers a lookup it refuses with an `MT_IMPORT` reply whose `exception_message` carries the reason and whose `connection_information` is absent. The session stays open. The reason tells a missing object (`connection <NAME> does not exist`) from a missing `ACCESS` grant (`insufficient privileges for using connection <NAME> in script <SCRIPT>`). The database checks the grant of the invoking user.

## Scenarios

### Scenario: connection fetches named-connection credentials via on-demand MT_IMPORT

* *GIVEN* a context whose dispatch loop is blocked awaiting the UDF function or single-call hook return, so the ZMQ socket is idle
* *WHEN* the UDF calls `ctx.connection("CREDS_CONN")`
* *THEN* the host MUST send an `MT_IMPORT` request with `kind = PB_IMPORT_CONNECTION_INFORMATION` and `script_name = "CREDS_CONN"`
* *AND* it MUST map the returned `connection_information_rep` (`kind`, `address`, `user`, `password`) into a `ConnectionObject` and return it to the UDF
* *AND* it MUST NOT open any `exarrow-rs` session, because the lookup is a pure credential retrieval
* *AND* it MUST behave identically whether or not the runtime is built with the `connect-back` feature

### Scenario: A refused CONNECTION lookup surfaces the database's reason

* *GIVEN* a UDF that calls `ctx.connection(name)` from `run` or from a single-call hook
* *WHEN* the database refuses the lookup, because the `CONNECTION` does not exist or the invoking user lacks `ACCESS` on it
* *THEN* `connection` MUST return `UdfError::Connection` whose text is the reply's `exception_message` verbatim
* *AND* `exception_message` MUST take precedence over any `connection_information` in the same reply
* *AND* a failure without a database reason (a transport error, an `MT_CLOSE`, an unexpected reply) MUST return `UdfError::Connection` with text `CONNECTION <name>: <reason>`, carrying an `MT_CLOSE` exception message when present
* *AND* no lookup error text MUST mention connect-back or repeat a `Protocol error:` prefix

### Scenario: A UDF recovers from a refused CONNECTION lookup

* *GIVEN* a UDF whose `ctx.connection(name)` call was refused
* *WHEN* the UDF handles the error and continues, for example by looking up another `CONNECTION` or returning a value
* *THEN* the session MUST stay usable, and a later lookup or the UDF's result MUST complete normally
