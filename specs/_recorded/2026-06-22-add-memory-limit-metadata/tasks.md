# Tasks: add-memory-limit-metadata

## Phase 2: Implementation (Group A — parallel)
- [x] 2.1 wire-protocol decode: add `maximal_memory_limit` to `UdfMeta` + `from_pb` + unit test
- [x] 2.2 sdk accessor: defaulted `UdfContext::memory_limit()` + unit test

## Phase 2: Implementation (Group B — after A)
- [x] 2.3 runtime wiring: `HostContextBridge.memory_limit` field + impl + unit test

## Phase 3: Verification
- [ ] 3.1 Build / test / clippy / fmt checklist
