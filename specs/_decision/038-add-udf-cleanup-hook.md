# Decisions: add-udf-cleanup-hook

## ADR: Repurpose the destroy slot as an optional cleanup hook with the run slot's shape

**ID:** repurpose-destroy-as-cleanup-slot
**Plan:** add-udf-cleanup-hook
**Status:** Accepted

### Context

The `ExaUdfVTable.destroy` slot takes no context and reports no error. The runtime calls it once per session, after the final wire message, so no error from it can reach the database. The reference client runs the user's `cleanup` inside `vm->shutdown()` before that final message, so its error does reach the database. The SDK needs a slot with the same signature the `run` slot already carries, because that shape supplies the context pointer and the error out-pointer cleanup needs. The struct field type change forces an ABI bump under every option considered.

### Decision

`ExaUdfVTable.destroy: unsafe extern "C" fn()` becomes `cleanup: Option<unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32>` at the same struct position. The slot is `None` unless the annotation carries `cleanup(path)`. The hook function is `fn(&mut dyn UdfContext) -> Result<(), UdfError>` and receives no outcome flag. `EXA_UDF_ABI_VERSION` goes `10 → 11`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Repurpose `destroy` with the `run` slot's shape, renamed `cleanup` | ✓ Chosen — one slot and one name per lifecycle concept, and the field type change forces the ABI bump under every option |
| Add a new slot beside `destroy` | ✗ Rejected — leaves a dead field and two lifecycle slots for one concept |
| Keep the field name `destroy` | ✗ Rejected — gives one concept two names, because the annotation, the SDK docs, and the issue call it `cleanup` |
| Add an outcome flag to the hook signature | ✗ Rejected — goes beyond Python/Java parity, and the user declined it |

### Consequences

The field is renamed `destroy → cleanup`, so the annotation, the vtable field, the loader method, and the shim share one name. The macro generates `__exa_cleanup_shim_<NAME>` only when annotated, through one shim builder shared with the `run` shim, so both slots map `Ok`, `Err`, and panic to `0`, `1`, and `2` by construction. The loader reads the return code and the error out-pointer of both slots through one helper, so both surface `UDF <slot> returned error code <rc>: <text>`. Hand-written vtables (`test-udfs/single-call-fixture`, the `cargo-exasol-udf` `VTableProbe` mirror, the loader test templates) rename the field and set `None`.

## ADR: The cleanup hook runs whenever dispatch started, before the final message, owned by one teardown step

**ID:** cleanup-runs-once-per-session-teardown
**Plan:** add-udf-cleanup-hook
**Status:** Accepted

### Context

The reference client calls `vm->shutdown()` before `MT_FINISHED` on the normal path, and in `handle_error(shutdown_vm=true)` for every exception after VM construction: single-call errors, run errors, and caught exceptions including a DB `MT_CLOSE`. The Rust runtime's two dispatchers (`dispatch::run_udf` and `single_call::run_single_call`) return through several exit paths, including `?`-propagated protocol errors, a DB `MT_CLOSE`, a mid-group `MT_CLEANUP`, run errors, and single-call hook errors. A cleanup call placed at each exit site would repeat the same "cleanup, then final message" decision at every site.

### Decision

`dispatch::run_udf` and `single_call::run_single_call` return `Ok(())` when the DB sends `MT_CLEANUP` and `Err` when an error ends dispatch. Neither sends `MT_FINISHED` or the error `MT_CLOSE`. `Runtime::run` then runs the cleanup hook through a new `cleanup` module and sends `MT_FINISHED` on success or one error `MT_CLOSE` otherwise. A failure before dispatch (load, ABI check, output-shape check, annotated-schema check) skips the hook.

### Options Considered

| Option | Verdict |
|--------|---------|
| One teardown step in `Runtime::run` after either dispatcher returns | ✓ Chosen — makes "every exit runs cleanup" a structural property, not a per-site discipline |
| A hook call at each exit site inside both dispatchers | ✗ Rejected — the exits include `?`-propagated protocol errors, a DB `MT_CLOSE`, a mid-group `MT_CLEANUP`, run errors, and single-call hook errors, so both modules would repeat the same decision at every site |
| Keep the call after the final message, as `destroy` runs today | ✗ Rejected — the hook's error could never reach the DB |
| Run the hook after a pre-dispatch validation failure | ✗ Rejected — the reference client skips cleanup when VM construction fails |

### Consequences

An error followed by a cleanup error produces one `MT_CLOSE` carrying the original error first and the cleanup error second, the order the reference client's own close message keeps. A mid-group `MT_CLEANUP` now ends with the cleanup hook and `MT_FINISHED`, as every other `MT_CLEANUP` does; the engine only sends `MT_CLEANUP` in answer to `MT_RUN` or `MT_DONE`, so this path is defensive. `Runtime::run` drops all four `destroy` calls.

## ADR: Cleanup gets a dedicated CleanupContext that refuses CONNECTION lookups

**ID:** dedicated-cleanup-context-refuses-connection
**Plan:** add-udf-cleanup-hook
**Status:** Accepted

### Context

After the engine sends `MT_CLEANUP`, it accepts only `MT_FINISHED` or `MT_CLOSE` as the next client message, so an `MT_IMPORT` exchange during cleanup would desync or hang the session. The SDK uses one `UdfContext` trait for every context-taking call site: `run`, the virtual-schema adapter, the import and export spec hooks, and now cleanup. Per-method defaults let one FFI shim shape serve every hook, and that trait design stays. The call sites differ in wire-protocol legality: the single-call hooks run inside the `MT_CALL` dialogue, where `MT_IMPORT` is legal, while cleanup runs after `MT_CLEANUP`, where it is not.

### Decision

The hook receives a new `CleanupContext`, a `UdfContext` implementation beside `SingleCallContext` in `crates/exa-udf-runtime/src/rowset.rs`. It carries the `MT_META` handshake metadata and the iteration axes, and it holds no credential requester. `ctx.connection(name)` returns `UdfError::ConnectBack` immediately and sends no `MT_IMPORT`. The error text states that CONNECTION lookups are unavailable during cleanup, and it tells the author to resolve the `ConnectionObject` during `run()` and keep it, for example in a `static`. The handshake accessors, `ctx.cluster_ip()`, and `ctx.connect_back(&conn)` behave as in `SingleCallContext`. `next`, `get`, and `emit` return errors.

### Options Considered

| Option | Verdict |
|--------|---------|
| A dedicated `CleanupContext` type with its own `connection` refusal | ✓ Chosen — a separate type encodes the phase rule in the type system and gives the rule one owner |
| Reuse `SingleCallContext` with its live `MT_IMPORT` requester | ✗ Rejected — after `MT_CLEANUP` the engine rejects every message except `MT_FINISHED` and `MT_CLOSE`, so an `MT_IMPORT` there fails the statement with an engine internal error, not the author's text, and the client waits for a reply that never arrives |
| `SingleCallContext` with a credential requester that refuses during cleanup | ✗ Rejected — one type then serves two protocol phases with different legality, and the difference lives in a closure the call site passes, not in the type |
| A live `MT_IMPORT` requester only when cleanup follows a `run()` error | ✗ Rejected — the same author code then works after a run error and fails after a successful run, and after a DB `MT_CLOSE` the control channel is closed, so the error path has no uniform rule either |
| A session-scoped cache of the CONNECTIONs resolved during `run()` | ✗ Rejected — adds a host-side credential store that lives for the whole session, and a name that `run()` did not resolve still fails, so the author still needs the resolve-during-`run` rule |

### Consequences

`CleanupContext` reuses `delegate_handshake_meta!()`. `delegate_connect_back_hooks!` splits into a `connection` part and a `cluster_ip` plus `connect_back` part. `HostContextBridge` and `SingleCallContext` invoke both parts, and `CleanupContext` invokes only the second. The refusal is a method body of the type, defined without a `connect-back` feature gate; it is not a runtime flag and not a closure. `SingleCallContext` and `HostContextBridge` keep their behavior. The `cleanup` module constructs a `CleanupContext` and holds no credential logic. A future call site with its own protocol-phase legality gets its own `UdfContext` implementation, not a flag on an existing one. The design assumes that the engine accepts a connect-back login while it waits for the final message of the cleanup exchange; the live scenario `cleanup_connects_back_with_a_resolved_connection_object` verifies this assumption.
