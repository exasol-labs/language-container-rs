# Decisions: add-udf-cleanup-hook

## ADR: Repurpose the destroy slot as an optional cleanup hook with the run slot's shape

**ID:** repurpose-destroy-as-cleanup-slot
**Plan:** add-udf-cleanup-hook
**Status:** Accepted

### Context

The `ExaUdfVTable.destroy` slot takes no context and reports no error, and the runtime calls it after the final wire message, so its errors cannot reach the database. The reference client runs the user's `cleanup` before that message. Cleanup needs the context pointer and error out-pointer that the `run` slot's shape supplies.

### Decision

`ExaUdfVTable.cleanup` is `Option<unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32>`, at the struct position of the former `destroy` field. The slot is `None` unless the annotation carries `cleanup(path)`. The hook is `fn(&mut dyn UdfContext) -> Result<(), UdfError>` and receives no outcome flag.

### Options Considered

| Option | Verdict |
|--------|---------|
| One `cleanup` slot with the `run` slot's shape | ✓ Chosen |
| New slot beside `destroy` | ✗ Leaves a dead field and two lifecycle slots for one concept |
| Keep the field name `destroy` | ✗ Annotation and docs call it `cleanup`, so one concept gets two names |
| Add an outcome flag to the hook | ✗ Exceeds Python/Java parity |

### Consequences

The annotation, vtable field, loader method and shim share the name `cleanup`. The macro generates `__exa_cleanup_shim_<NAME>` only when annotated, through the shim builder the `run` shim also uses, so both slots map `Ok`, `Err` and panic to `0`, `1` and `2`. The loader reads both slots' return code and error out-pointer through one helper, so both surface `UDF <slot> returned error code <rc>: <text>`. Hand-written vtables (`test-udfs/single-call-fixture`, the `cargo-exasol-udf` `VTableProbe` mirror, the loader test templates) set `cleanup` to `None`. The field type change bumps `EXA_UDF_ABI_VERSION`.

## ADR: The cleanup hook runs whenever dispatch started, before the final message, owned by one teardown step

**ID:** cleanup-runs-once-per-session-teardown
**Plan:** add-udf-cleanup-hook
**Status:** Accepted

### Context

The reference client runs cleanup before `MT_FINISHED` on the normal path and on every error after VM construction. The two dispatchers (`dispatch::run_udf` and `single_call::run_single_call`) have many exit paths, so a call at each exit would repeat the same decision at every site.

### Decision

`dispatch::run_udf` and `single_call::run_single_call` return `Ok(())` when the DB sends `MT_CLEANUP` and `Err` when an error ends dispatch. Neither sends `MT_FINISHED` or the error `MT_CLOSE`. `Runtime::run` then runs the cleanup hook through the `cleanup` module and sends `MT_FINISHED` on success or one error `MT_CLOSE` otherwise. A failure before dispatch (load, ABI check, output-shape check, annotated-schema check) skips the hook.

### Options Considered

| Option | Verdict |
|--------|---------|
| One teardown step in `Runtime::run` | ✓ Chosen |
| A hook call at each exit site in both dispatchers | ✗ Repeats the decision at every exit |
| Call after the final message | ✗ The hook's error cannot reach the DB |
| Run the hook after a pre-dispatch failure | ✗ The reference client skips cleanup when VM construction fails |

### Consequences

An error followed by a cleanup error produces one `MT_CLOSE` with the original error first and the cleanup error second. A mid-group `MT_CLEANUP` ends with the cleanup hook and `MT_FINISHED`. The engine sends `MT_CLEANUP` only in answer to `MT_RUN` or `MT_DONE`, so this path is defensive. `Runtime::run` makes no `destroy` calls.

## ADR: Cleanup gets a dedicated CleanupContext that refuses CONNECTION lookups

**ID:** dedicated-cleanup-context-refuses-connection
**Plan:** add-udf-cleanup-hook
**Status:** Accepted

### Context

After `MT_CLEANUP` the engine accepts only `MT_FINISHED` or `MT_CLOSE`, so an `MT_IMPORT` exchange during cleanup desyncs or hangs the session. Single-call hooks run inside the `MT_CALL` dialogue where `MT_IMPORT` is legal. All call sites share the `UdfContext` trait with per-method defaults.

### Decision

The hook receives `CleanupContext`, a `UdfContext` implementation beside `SingleCallContext` in `crates/exa-udf-runtime/src/rowset.rs`. It carries the `MT_META` handshake metadata and the iteration axes, and holds no credential requester. `ctx.connection(name)` returns `UdfError::ConnectBack` immediately and sends no `MT_IMPORT`. The error text states that CONNECTION lookups are unavailable during cleanup and tells the author to resolve the `ConnectionObject` during `run()` and keep it, for example in a `static`. The handshake accessors, `ctx.cluster_ip()` and `ctx.connect_back(&conn)` behave as in `SingleCallContext`. `next`, `get` and `emit` return errors.

### Options Considered

| Option | Verdict |
|--------|---------|
| Dedicated `CleanupContext` with its own `connection` refusal | ✓ Chosen |
| Reuse `SingleCallContext` with its live `MT_IMPORT` requester | ✗ The engine rejects `MT_IMPORT` after `MT_CLEANUP` and the client waits for a reply that never arrives |
| `SingleCallContext` with a requester that refuses during cleanup | ✗ One type serves two protocol phases and the difference hides in a closure |
| Live requester only when cleanup follows a `run()` error | ✗ Author code behaves differently after an error and after success, and a DB `MT_CLOSE` closes the channel |
| Session-scoped cache of CONNECTIONs resolved during `run()` | ✗ Adds a session-long credential store, and unresolved names still fail |

### Consequences

`CleanupContext` reuses `delegate_handshake_meta!()`. `delegate_connect_back_hooks!` splits into a `connection` part and a `cluster_ip` plus `connect_back` part. `HostContextBridge` and `SingleCallContext` invoke both parts, and `CleanupContext` invokes only the second. The refusal is a method body without a `connect-back` feature gate. A call site with its own protocol-phase legality gets its own `UdfContext` implementation. The design assumes the engine accepts a connect-back login while it waits for the final cleanup message. The live scenario `cleanup_connects_back_with_a_resolved_connection_object` verifies this.
