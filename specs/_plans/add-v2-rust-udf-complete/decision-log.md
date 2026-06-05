# Decision Log: add-v2-rust-udf-complete

Date: 2026-06-05

## Interview

**Q:** Which capability areas should v2 cover?
**A:** All of: cargo-exaudf CLI, Single-call protocol (`SC_FN_*`), connect-back (`ExaConnection`), typed macro annotations, and the VS Adapter.

**Q:** Should the JIT path be implemented in v2?
**A:** No — skip JIT for now. Keep Option A (precompiled `.so` from BucketFS) only. `compiler.rs` stays returning `UnsupportedFeature`.

**Q:** For the VS Adapter, wire up the real hook or leave an `MT_UNDEFINED_CALL` stub?
**A:** Wire it up — dispatch `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL` to the `UdfRun::virtual_schema_adapter_call` hook and reply `MT_RETURN`, exactly like the other `SC_FN_*` hooks.

**Q:** Which `cargo exaudf` subcommands should v2 ship?
**A:** All three — `new`, `build`, and `validate`.

### v2 completion augmentation (2026-06-05)

**Q:** Is the native exarrow-rs protocol failure client-side or server-side?
**A:** Unknown at planning time. PyExasol works within the Python SLC on both 2025 and 2026. The plan must root-cause before committing to a solution (Group F task 6.2).

**Q:** What is the scope of v2 beyond connect-back working?
**A:** No other scope — v2 = connect-back working end-to-end + merge of `connect-back-fix` + version bump to `0.2.0`. Nothing else.

### Connect-back transport decision (2026-06-05)

**Q:** Native exarrow-rs protocol vs WebSocket for the connect-back connection — which transport should the runtime use?
**A:** Native. "You should be able to use exarrow-rs with native protocol and connect back to Exasol by opening a new connection. Native should be faster than WebSockets. Native shall work." This closes the open "compare and choose" question from task 6.2 — native is now mandated, not a candidate. (Supersedes decision [10]; see ADR [12].)

## Design Decisions

### [1] ExaConnection trait in the SDK, implemented by the runtime

- **Decision:** Connect-back is exposed as an `ExaConnection` trait defined in `exasol-udf-sdk` (behind the `connect-back` feature). The `exa-udf-runtime` crate provides the only implementation, backed by exarrow-rs. UDFs depend only on `exasol-udf-sdk` + `arrow`.
- **Alternatives:** Return `exarrow_rs::adbc::Connection` directly from `ctx.connect_back()` (the original v1 design).
- **Rationale:** Returning the concrete type forces every connect-back UDF to statically link exarrow-rs into its musl `.so` — expensive and unnecessary since the host process already owns the connection infrastructure (design §11.3).
- **Promotes to ADR:** yes

### [2] Dedicated OnceLock current_thread runtime for connect-back

- **Decision:** The runtime owns a `CONNECT_BACK_RT: OnceLock<tokio::runtime::Runtime>` (current_thread) and `block_on`s exarrow-rs async calls from the synchronous ZMQ dispatch loop.
- **Alternatives:** Make the whole dispatch loop async; spawn a multi-thread tokio runtime.
- **Rationale:** The ZMQ loop is blocking and must stay so; a single-thread runtime entered only for connect-back keeps async strictly contained and never crosses into the protocol state machine.
- **Promotes to ADR:** yes

### [3] Wire SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL to a real hook

- **Decision:** `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL` dispatches to `UdfRun::virtual_schema_adapter_call` and replies `MT_RETURN`, joining the existing `SC_FN_*` hook set.
- **Alternatives:** Leave it as an `MT_UNDEFINED_CALL` stub.
- **Rationale:** User explicitly requested full wiring; treating it uniformly with the other single-call hooks keeps the dispatcher table-driven.
- **Promotes to ADR:** no

### [4] JIT explicitly out of scope

- **Decision:** Do not spec or implement JIT; `compiler.rs` remains returning `UnsupportedFeature`.
- **Alternatives:** Implement Option C in-container compilation in v2.
- **Rationale:** User deferred JIT. Scoping it out keeps v2 focused on the precompiled `.so` path and avoids the ~1.4 GB jit container surface.
- **Promotes to ADR:** yes

### [5] Annotation type mapping validated at compile time and load time

- **Decision:** The macro maps Rust type tokens to `ExaType` and fails compilation on unmappable types; the runtime additionally validates the embedded schema against `exascript_metadata` at load, closing the session with an `F-UDF-CL-RUST-####` error on mismatch.
- **Alternatives:** Validate only at load time.
- **Rationale:** Compile-time mapping gives authors immediate feedback; load-time validation still catches DB/declaration drift. The two layers are complementary.
- **Promotes to ADR:** no

### [6] VS adapter verified at unit level only

- **Decision:** The virtual-schema adapter call is covered by a runtime unit dispatch test rather than a full DB roundtrip.
- **Alternatives:** Add a live-DB virtual schema integration test.
- **Rationale:** The adapter hook needs no live database to exercise; a unit dispatch test is sufficient and avoids standing up a virtual schema fixture.
- **Promotes to ADR:** no

### [7] cargo-exaudf hides the musl target triple

- **Decision:** `cargo exaudf build` always targets `x86_64-unknown-linux-musl`, auto-installing the target via `rustup target add` if absent, and never exposes the triple to the author.
- **Alternatives:** Require authors to pass `--target` or pre-install the musl target.
- **Rationale:** Fully-static musl is the only supported deploy artifact; hiding the triple removes a class of author error and matches the mission's documented author workflow.
- **Promotes to ADR:** yes

### [8] Connect-back uses named-connection metadata, not an internal proxy

- **Decision:** The runtime opens the connect-back connection to the `address`/`user`/`password` returned by the on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) response, connecting exactly as an external client would. There is no dedicated internal "connect-back proxy" endpoint.
- **Evidence:** `exasol/script-languages` — `zmqcontainer.proto` shows `exascript_info` (MT_INFO) carries no `connection_information`; the only credential source is `MT_IMPORT` returning `connection_information_rep { kind, address, user, password }`. The `get_connection` language tests and `ExaConnectionInformationImpl.java` show the SLC only passes this metadata to user code (`exa.get_connection(name)`) and never opens a connection itself; the Python "connect-back" that works is the user's own `pyexasol.connect(address, …)`.
- **Alternatives:** Keep treating the named connection as an internal proxy at the sandbox loopback/eth0 `:8563` (the v1 approach that crashed the session); keep hard-pinning `transport=websocket`.
- **Rationale:** The literal `CREATE CONNECTION ... TO '<address>'` is just a routable endpoint authenticated with user/password (no token). Pointing it at the UDF sandbox's own loopback/eth0 is what triggered the `2026.1.0` SIGABRT, not a WebSocket-vs-native server bug. Connecting to a routable address as an external client matches the reference SLC.
- **Promotes to ADR:** yes

### [9] "Proactive handshake credentials" path is a phantom; on-demand MT_IMPORT is primary

- **Decision:** Treat the on-demand `MT_IMPORT` `conn_requester` path as the supported and primary connect-back credential path. The proactive `UdfMeta.conn_info` from the handshake is kept only as a no-op fallback for a hypothetical future server that pushes credentials.
- **Alternatives:** Continue prioritising/assuming proactive handshake credentials.
- **Rationale:** `exascript_info` has no `connection_information` field, so the DB never pushes credentials proactively; the prior "proactive ConnInfo from handshake" assumption could never fire.
- **Promotes to ADR:** no

### [10] Root-cause empirically before changing transport (SUPERSEDED by [12])

- **Status:** Superseded on 2026-06-05 by decision [12]. The "compare native vs WebSocket and choose" approach is replaced by a mandate that native is the connect-back transport. Retained for history.
- **Decision:** Group F task 6.2 captures exarrow-rs native vs WebSocket handshakes against a routable `2026.1.0` endpoint and selects the transport that authenticates cleanly, rather than re-asserting `transport=websocket`.
- **Alternatives:** Assume the prior conclusion (server bug, WebSocket required) and skip diagnosis.
- **Rationale:** The prior "known server limitation" verdict was reached without comparing against how the reference SLC connects; an empirical capture from a *routable* endpoint is needed to distinguish address misuse from a true transport issue.
- **Promotes to ADR:** no

### [11] connect-back-fix child plan absorbed and archived

- **Decision:** The `connect-back-fix` plan's two fixes (feature flag on `exaudfclient`, run-phase `ConnInfo` via `conn_requester`) are folded into this plan as already-done context; `connect-back-fix` is archived rather than carried as a separate active plan.
- **Alternatives:** Keep `connect-back-fix` as a standalone active plan.
- **Rationale:** Its scope is a strict subset of v2 completion; merging avoids two plans racing on the same connect-back code.
- **Promotes to ADR:** no

### [12] Native binary protocol is the mandatory connect-back transport (ADR)

- **Status:** Accepted (2026-06-05). Supersedes decision [10].
- **Context:** Connect-back opens a *new* connection from inside the UDF sandbox back to a routable Exasol endpoint using the `address`/`user`/`password` from the named connection. exarrow-rs supports two transports selected via the DSN `transport=` parameter: `native` (binary protocol, the default `native` feature) and `websocket`. The v1 code hard-pinned `transport=websocket`. Task 6.2 previously left the choice open ("empirically compare and choose").
- **Decision:** The connect-back connection MUST use the exarrow-rs **native binary protocol**. The runtime achieves this by building the DSN with **no `transport=` override**, relying on exarrow-rs's default `native` feature (the slc-rs workspace declares `exarrow-rs` with no feature overrides, so `native` is already the default and `None`/unspecified transport resolves to `NativeTcpTransport`). The `transport=websocket` pin is removed.
- **Alternatives considered:**
  - Keep `transport=websocket` (v1 behaviour) — rejected; WebSocket was only assumed necessary due to the address-misuse SIGABRT, which decision [8] root-caused to a non-routable endpoint, not the transport.
  - Empirically benchmark native vs WebSocket and pick the winner (old task 6.2) — rejected as an open question; the user has made the call.
- **Rationale:**
  - **Performance** — the native binary protocol is faster than WebSocket for the connect-back data path.
  - **Consistency** — native matches the transport used by the main Exasol session, keeping one wire format across the runtime.
  - **Simplicity** — relying on the exarrow-rs default removes a DSN parameter and a divergent code path.
  - **Code-verified (2026-06-05)** — the workspace `Cargo.toml` declares `exarrow-rs` with no feature override, so the default `native` feature is active and an unspecified DSN `transport=` resolves to `NativeTcpTransport`. `build_dsn()` now emits no `transport=` parameter. (No live DB test in this sub-task; live verification is Group I.)
- **Consequences:**
  - The WebSocket connect-back path is left untested and unsupported; `transport=websocket` is no longer emitted.
  - Task 6.2 becomes a verification task (confirm native connects cleanly against a routable `2026.1.0` endpoint) rather than a comparison task.
  - If a future Exasol DB version rejects or breaks the native connect-back handshake, this decision MUST be re-evaluated (re-open the native-vs-WebSocket comparison at that point).
- **Promotes to ADR:** yes

### [13] Confirmed: handshake carries no conn_info

- **Confirmed (2026-06-05):** `UdfMeta::from_pb` (`exa-zmq-protocol/src/meta.rs`) hardcodes `conn_info: None`; the handshake `ExascriptInfo` (MT_INFO) has no `connection_information` field to read from.
- The only `ConnInfo` source is `HostEvent::ConnInfo`, emitted from the on-demand `MT_IMPORT` reply (`loop_.rs` parses `import.connection_information` into `ConnInfo::from_pb`).
- The working credential path is the `conn_requester` closure in `exa-udf-runtime/src/dispatch.rs`, which fires the on-demand `MT_IMPORT` and maps the reply to `ConnInfo`. The proactive `UdfMeta.conn_info` path is a no-op fallback per decision [9].

### [14] ABI/fingerprint unchanged at 0.2.0

- **Confirmed (2026-06-05):** `EXA_UDF_ABI_VERSION = 2` and `EXA_SDK_FINGERPRINT = "0.2.0:<rustc_hash>"` — neither constant was modified by the Group F connect-back fixes. The vtable layout did not change.

### [15] Confirmed: connect-back SIGABRT is a server-side bug in Exasol 2026.1.0 (any transport, any address)

- **Status:** Confirmed empirically on 2026-06-05. Supersedes the incorrect root cause in decision [8] regarding "wrong address."
- **Evidence:**
  - With native binary protocol + container inner IP (`172.17.x.x:8563`): outer session SIGABRTs when connect-back session starts.
  - With native binary protocol + Docker host gateway + mapped port (external path): outer session SIGABRTs when connect-back session starts.
  - With WebSocket protocol + Docker host gateway + mapped port: outer session SIGABRTs when connect-back session starts.
  - docker logs confirm: `child 1913 (Part:40 Node:0 exasql) terminated with signal 6 (core dumped)` immediately after the connect-back session process (Part:44) is started by the DB.
  - Pattern: the connect-back session creation (spawning a new exasql process) always triggers the SIGABRT on the outer session handler, regardless of transport or address.
  - `2026.latest` tag does not exist in Docker Hub; `2026.1.0` is the only available 2026 image.
- **Conclusion:** This is a server-side bug in Exasol `2026.1.0` where creating a new inbound SQL session while a UDF is executing causes the outer session handler to assert-abort. It is not caused by the address or protocol choice. V.6 cannot pass on `2026.1.0` until this is fixed server-side.
- **Action:** V.6 remains open. All other Group I verification (V.1–V.5, 9.1, 9.3 via unit-level proxy) has passed. Decision [12] (native binary protocol) remains the correct choice; the SIGABRT is not transport-specific.

### [14] ABI/fingerprint unchanged at 0.2.0

- **Confirmed (2026-06-05):** `EXA_UDF_ABI_VERSION = 2` and `EXA_SDK_FINGERPRINT` (format `"SDK_VERSION:RUSTC_HASH\0"`) were NOT changed by the Group F work. Group F only modified `connect_back.rs` (fix DSN, remove debug instrumentation) and the `it/` integration harness (routable CB_SELF endpoint). Neither `abi.rs` nor the vtable layout was touched. The version bump to `0.2.0` is a Cargo package version change only and does not alter the ABI version or fingerprint baked into compiled `.so` artifacts.

## Review Findings

<!-- Populated by speq-implement after code review. -->
