# Tasks: 2026-06-11-vs-adapter-and-single-call-connect-back

All tasks are verification-only — the code already exists and is tested.

## Verification Tasks

- [x] 1.1 Verify spec delta for `sdk/udf-sdk` matches `abi.rs` and `lib.rs` changes (ABI v3, vs_adapter slot signature, macro shim generation)
- [x] 1.2 Verify spec delta for `sdk/connect-back` matches `connect_back.rs` trait additions (`begin`/`commit`/`rollback` defaults)
- [x] 1.3 Verify spec delta for `protocol/wire-protocol` matches `loop_.rs`/`messages.rs` (`SingleCallAck` event, `MT_RETURN` in single-call mode)
- [x] 1.4 Verify spec delta for `runtime/connect-back` matches `connect_back.rs` (`run_txn_op`, `SingleCallContext` implementation) [expert]
- [x] 1.5 Verify spec delta for `runtime/host-dispatch` matches `rowset.rs` (row-major packing, no placeholders, declared-type dispatch) [expert]
- [x] 1.6 Run `cargo +1.91 test --workspace` and confirm green
- [x] 1.7 Run `cargo clippy --all-targets --all-features -- -D warnings` and confirm clean
- [ ] 1.8 Bump `Cargo.toml` workspace version from `0.4.0` to `0.5.0` and update `Cargo.lock`
- [ ] 1.9 Run `speq plan validate 2026-06-11-vs-adapter-and-single-call-connect-back` and confirm pass
- [ ] 1.10 Commit all changes with message `feat!: ABI v3, vs_adapter macro, single-call loop fix, row-major rowset`
