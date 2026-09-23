# Decision Log: add-udf-cleanup-hook

## Interview

**Q:** The vtable already has an unconditional `destroy: extern "C" fn()` slot that the macro always fills with a no-op, called once per session today but too late in the wire sequence for any error to reach the DB. Should the new cleanup hook repurpose that slot, or live beside it as a new slot?
**A:** Repurpose `destroy`. Change its signature to `(ctx, error_out) -> i32` and make it `Option<...>` (`None` when the author defines no cleanup). One slot for the one lifecycle concept it already names, and no dead field left behind. The ABI bumps either way, because the struct field type changes under both options.

**Q:** What should the cleanup function have access to? The runtime already has `SingleCallContext`, a lightweight `UdfContext` implementation (handshake metadata and connect-back, no row or emit access) that the virtual-schema-adapter and import/export-spec hooks use.
**A:** Reuse `SingleCallContext` as-is: metadata accessors plus `ctx.connection(...)` and `ctx.connect_back()` for reference-data lookups during cleanup. No new context type. This matches the pattern every other context-taking single-call hook uses.

**Q:** Should the cleanup hook go beyond Python/Java parity and pass a success/error outcome flag?
**A:** No outcome flag. This matches Python and Java exactly: cleanup runs identically regardless of how the session ended, with the simplest signature.

**Q:** `ctx.connection(name)` cannot work inside cleanup. After the engine sends `MT_CLEANUP`, it accepts only `MT_FINISHED` or `MT_CLOSE`, so the `MT_IMPORT` exchange would desync or hang the session. Which resolution applies: (a) a `SingleCallContext` whose credential lookup always refuses during cleanup, (b) `connection()` only when cleanup follows a `run()` error, or (c) a cache of the CONNECTIONs resolved during `run()` that cleanup serves?
**A:** None of them. Cleanup gets its own `CleanupContext` type, a sibling of `SingleCallContext`, not a reuse of it. It still implements `UdfContext`, so the FFI shim shape stays unchanged, and it delegates the metadata accessors and `connect_back` the way `SingleCallContext` does. Its `connection(name)` refuses immediately and sends no `MT_IMPORT`. The error tells the author to resolve the `ConnectionObject` during `run()` and keep it, for example in a `static`. The refusal is a permanent, documented property of the type, not a runtime special case.

## Design Decisions

### [1] Repurpose the destroy slot as an optional cleanup hook with the run slot's shape

- **Decision:** `ExaUdfVTable.destroy: unsafe extern "C" fn()` becomes `cleanup: Option<unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32>` at the same struct position. The slot is `None` unless the annotation carries `cleanup(path)`. The hook function is `fn(&mut dyn UdfContext) -> Result<(), UdfError>` and receives no outcome flag. `EXA_UDF_ABI_VERSION` goes `10 → 11`.
- **Alternatives:** A new slot beside `destroy` leaves a dead field and two lifecycle slots for one concept. Keeping the field name `destroy` gives one concept two names, because the annotation, the SDK docs, and the issue call it `cleanup`. An outcome flag goes beyond parity, and the user declined it.
- **Rationale:** The `run` slot shape already carries the context pointer and the error out-pointer that cleanup needs. The field type change forces the ABI bump under every option, and the minor SDK version bump already forces downstream rebuilds.
- **Consequences:**
  - The field is renamed `destroy → cleanup`, so the annotation, the vtable field, the loader method, and the shim share one name.
  - The macro generates `__exa_cleanup_shim_<NAME>` only when annotated, through one shim builder shared with the `run` shim, so both slots map `Ok`, `Err`, and panic to `0`, `1`, and `2` by construction.
  - The loader reads the return code and the error out-pointer of both slots through one helper, so both surface `UDF <slot> returned error code <rc>: <text>`.
  - Hand-written vtables (`test-udfs/single-call-fixture`, the `cargo-exasol-udf` `VTableProbe` mirror, the loader test templates) rename the field and set `None`.
- **Promotes to ADR:** yes

### [2] The cleanup hook runs whenever dispatch started, before the final message, owned by one teardown step

- **Decision:** `dispatch::run_udf` and `single_call::run_single_call` return `Ok(())` when the DB sends `MT_CLEANUP` and `Err` when an error ends dispatch. Neither sends `MT_FINISHED` or the error `MT_CLOSE`. `Runtime::run` then runs the cleanup hook through a new `cleanup` module and sends `MT_FINISHED` on success or one error `MT_CLOSE` otherwise. A failure before dispatch (load, ABI check, output-shape check, annotated-schema check) skips the hook.
- **Alternatives:** Invoking the hook inside both dispatchers at each exit site was rejected. The exits include `?`-propagated protocol errors, a DB `MT_CLOSE`, a mid-group `MT_CLEANUP`, run errors, and single-call hook errors, so both modules would repeat the "cleanup, then final message" decision at every site. Keeping the call after the final message (today's `destroy`) was rejected, because the hook's error could never reach the DB. Running the hook after a pre-dispatch validation failure was rejected, because the reference client skips cleanup when VM construction fails (`exaudflib_main.cc:195-202`, `shutdown_vm=false`).
- **Rationale:** The reference client calls `vm->shutdown()` before `MT_FINISHED` on the normal path (`exaudflib_main.cc:270-277`). It also calls it in `handle_error(shutdown_vm=true)` for every exception after VM construction: single-call errors (`:240`), run errors (`:262`), and caught exceptions including a DB `MT_CLOSE` (`:279-286`). One teardown owner makes "every exit" a structural property, not a per-site discipline.
- **Consequences:**
  - An error followed by a cleanup error produces one `MT_CLOSE` carrying the original error first and the cleanup error second, the order of the reference `F-UDF-CL-LIB-1111` message (`exaudflib_main.cc:71`).
  - A mid-group `MT_CLEANUP` now ends with the cleanup hook and `MT_FINISHED`, as every other `MT_CLEANUP` does. The engine only sends `MT_CLEANUP` in answer to `MT_RUN` or `MT_DONE`, so this path is defensive. `mid_group_cleanup_ends_session_cleanly` updates to answer `MT_FINISHED`.
  - `Runtime::run` drops all four `destroy` calls.
- **Promotes to ADR:** yes

### [3] Cleanup gets a dedicated CleanupContext that refuses CONNECTION lookups

- **Decision:** The hook receives a new `CleanupContext`, a `UdfContext` implementation beside `SingleCallContext` in `crates/exa-udf-runtime/src/rowset.rs`. It carries the `MT_META` handshake metadata and the iteration axes, and it holds no credential requester. `ctx.connection(name)` returns `UdfError::ConnectBack` immediately and sends no `MT_IMPORT`. The error text states that CONNECTION lookups are unavailable during cleanup, and it tells the author to resolve the `ConnectionObject` during `run()` and keep it, for example in a `static`. The handshake accessors, `ctx.cluster_ip()`, and `ctx.connect_back(&conn)` behave as in `SingleCallContext`. `next`, `get`, and `emit` return errors.
- **Alternatives:**
  - Reuse `SingleCallContext` with its live `MT_IMPORT` requester: rejected. After `MT_CLEANUP` the engine reads one more message and rejects every type except `MT_FINISHED` and `MT_CLOSE`. An `MT_IMPORT` there fails the statement with an engine internal error, not the author's text, and the client waits for a reply that never arrives.
  - (a) `SingleCallContext` with a credential requester that refuses during cleanup: rejected. One type then serves two protocol phases with different legality. The difference lives in a closure the call site passes, not in the type, so `connection()` on one type identity works in one phase and fails in the other.
  - (b) A live `MT_IMPORT` requester only when cleanup follows a `run()` error: rejected. The same author code then works after a run error and fails after a successful run. After a DB `MT_CLOSE` the control channel is closed, so the error path has no uniform rule either.
  - (c) A session-scoped cache of the CONNECTIONs resolved during `run()`: rejected. It adds a host-side credential store that lives for the whole session. A name that `run()` did not resolve still fails, so the author still needs the resolve-during-`run` rule.
  - A dedicated `CleanupContext` type: chosen.
- **Rationale:** The SDK uses one `UdfContext` trait for every context-taking call site: `run`, the virtual-schema adapter, the import and export spec hooks, and now cleanup. Per-method defaults (`Err(Unimplemented)`) let one FFI shim shape serve every hook: `&mut dyn UdfContext` behind a double-indirected pointer, within the vtable-stability rules of `sdk/udf-abi`. That trait design stays. The call sites differ in wire-protocol legality. The single-call hooks run inside the `MT_CALL` dialogue, where `MT_IMPORT` is legal. Cleanup runs after `MT_CLEANUP`, where the engine accepts only `MT_FINISHED` or `MT_CLOSE` (engine source: the cleanup exchange in `zmqinternal.cc`, the same rule in `zmqexternal.cc`). One concrete type for both call sites hides that difference behind one type identity. A separate type encodes the phase rule in the type system and gives the rule one owner. `connect_back` opens a TCP login and sends nothing on the control channel, so it has no phase-legality problem.
- **Consequences:**
  - `CleanupContext` reuses `delegate_handshake_meta!()`. `delegate_connect_back_hooks!` splits into a `connection` part and a `cluster_ip` plus `connect_back` part. `HostContextBridge` and `SingleCallContext` invoke both parts, and `CleanupContext` invokes only the second.
  - The refusal is a method body of the type, defined without a `connect-back` feature gate. It is not a runtime flag and not a closure. `SingleCallContext` and `HostContextBridge` keep their behavior.
  - The `cleanup` module constructs a `CleanupContext` and holds no credential logic.
  - A future call site with its own protocol-phase legality gets its own `UdfContext` implementation, not a flag on an existing one.
  - The design assumes that the engine accepts a connect-back login while it waits for the final message of the cleanup exchange. The live scenario `cleanup_connects_back_with_a_resolved_connection_object` verifies this assumption.
  - The last interview answer replaces the earlier answer to reuse `SingleCallContext` as-is.
- **Promotes to ADR:** yes

### [4] No UdfRun::cleanup trait method

- **Decision:** `UdfRun` gains no `cleanup` method. The author wires the hook only through the `cleanup(path)` annotation section.
- **Alternatives:** A defaulted `UdfRun::cleanup` was rejected. The macro reads hook functions from annotation paths, not from the trait, so the method would be a second declaration that nothing calls.
- **Rationale:** `sdk/udf-spec-hooks` scopes `UdfRun` to one hook per single-call function id, and cleanup is not a single-call function.
- **Promotes to ADR:** no

## Review Findings
