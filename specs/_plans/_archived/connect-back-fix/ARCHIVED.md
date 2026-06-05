# ARCHIVED

This plan was absorbed into `add-v2-rust-udf-complete` on 2026-06-05.

Its two implemented fixes (the `connect-back` feature flag on `exaudfclient` and run-phase
`ConnInfo` capture via the `conn_requester` MT_IMPORT closure in `dispatch.rs`) are recorded
there as already-done context. Its blocked verification (the connect-back session SIGABRT on
`2026.1.0`) was re-opened and root-caused in `add-v2-rust-udf-complete` Group F.

Do not resume this plan. See `specs/_plans/add-v2-rust-udf-complete/`.
