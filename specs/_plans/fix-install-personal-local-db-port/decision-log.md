# Decision Log: fix-install-personal-local-db-port

## Interview

**Q:** Should this plan cover just the core fix, or the core fix plus issue #83's optional follow-ups (print the resolved `host:port` in the "Registering" banner, decide `--port` behavior, extract local connection resolution into a testable function)?
**A:** Core fix only — narrowed by the next two answers to include a small `--port` fix and a minimal testability extraction. The banner enhancement is explicitly out of scope.

**Q:** Should `--port` start being honored as an override for local deployments (matching cloud), or remain silently ignored (current documented behavior)?
**A:** Honor `--port` for local too — make it symmetric with cloud: `PORT="${CLI_PORT:-$(deployment_field … '.connection.dbPort' 8563)}"`. This requires updating the `--help` usage text (line 104 currently says "ignored for local `--deployment`, honored for cloud `--deployment`"), which will no longer be true.

**Q:** Given core-fix-only scope (no broader test-harness refactor), how should the plan get regression coverage, since the local branch's connection logic is inlined in `main()` (lines 395-410) and untested — the reason this bug went unnoticed, as the sourced unit harness only exercises `resolve_cloud_connection`?
**A:** Minimal extraction for testability — extract just the local DB host/port resolution (not the full local connection flow: password and scope resolution stay where they are) into a small dedicated function analogous to `resolve_cloud_connection`, reusing the existing `deployment_field` helper the way cloud already does (local currently does not; its `deployment_ssh_port` even re-implements `deployment_field`'s pattern by hand). The function must be callable from `scripts/tests/install-personal-test.sh` in isolation, mirroring how `resolve_cloud_connection` is tested by `resolves_cloud_connection_from_descriptor`, `cli_flags_override_cloud_descriptor`, and `resolves_cloud_defaults_when_connection_fields_absent`.

**Q:** `specs/container/personal-install/spec.md`'s background asserts local Personal "publishes only the SQL port (`8563`)" and no local scenario requires reading `connection.dbPort` — how should the spec delta handle this?
**A:** Correct the background and add a local `dbPort` scenario — fix the incorrect assertion, and add a scenario for the local branch mirroring the cloud scenario "Cloud connection details resolve from the deployment directory": the DB port (and, per the `--port` decision, the override) MUST come from `connection.dbPort`, defaulting to `8563` when absent.

## Design Decisions

### [1] Extract `resolve_local_connection` rather than fixing the two lines in place

- **Decision:** Add `resolve_local_connection <dir>` beside `resolve_cloud_connection`, owning the local `HOST`/`PORT` decision, reading `.connection.dbPort` through `deployment_field`, honoring `CLI_PORT`, and returning (never exiting) so `scripts/tests/install-personal-test.sh` drives it in-process. `main`'s local branch calls it in a condition context (`|| die`).
- **Alternatives:** Fix in place — `PORT="$(deployment_field "$DEPLOYMENT_DIR" '.connection.dbPort' 8563)"` inside `main`. Rejected: the one-line form is behaviorally correct but stays unreachable from the sourced harness, which cannot run `main`. That unreachability is exactly why the defect shipped while every cloud connection value was pinned by an assertion.
- **Rationale:** The project already has this seam and tests through it; the fix costs one function and closes the coverage gap that hid the bug.
- **Promotes to ADR:** no

### [2] Extract host/port only; password, scope, SSH port, and node key stay in `main`

- **Decision:** `resolve_local_connection` owns the SQL host and port. The local branch keeps `LOCAL_TRANSPORT=1`, `SCOPE=SYSTEM`, the `secrets.json` password resolution and its `die`, the `deployment_ssh_port` read, and the node-key check inline in `main`.
- **Alternatives:** Extract the whole local branch so it mirrors `resolve_cloud_connection`'s fuller ownership (which includes password resolution and the presence checks). Rejected for this plan: it moves `die` calls and unrelated reads, turning a bug fix into a refactor with its own review surface.
- **Rationale:** The interview scoped the extraction to what the defect needs. The resulting local/cloud asymmetry — cloud resolves the password, local does not — is deliberate and recorded here so it is not read as an oversight; completing the mirror is a follow-up candidate.
- **Promotes to ADR:** no

### [3] Honor `--port` for a local deployment

- **Decision:** `resolve_local_connection` lets a non-empty `CLI_PORT` win over the descriptor value, matching `resolve_cloud_connection`. `usage()` and `docs/installation.md` drop the "ignored for local" claim.
- **Alternatives:** Keep discarding it (today's documented behavior) — rejected: silently overwriting an explicit flag is the same defect class as the hardcode, and it wasted the reporter's debugging time. Reject `--port` with an error for local — rejected: it adds a per-backend rule where symmetry costs nothing.
- **Rationale:** One precedence rule for `--port` across both backends is less to document and less to get wrong.
- **Promotes to ADR:** no

### [4] Read `.connection.dbPort` with no numeric validation

- **Decision:** Use `deployment_field "$dir" '.connection.dbPort' "$PERSONAL_DB_PORT_DEFAULT"`, which substitutes the default when the field is absent or empty and performs no numeric check.
- **Alternatives:** Validate numerically the way `deployment_ssh_port` does. Rejected: the cloud branch reads the same field with no validation, so validating on one side only would give one descriptor field two behaviors. A malformed value surfaces as an `exapump` connection error.
- **Rationale:** `deployment_field` is documented as the sole reader for every `.connection.*` field; one field, one parsing behavior.
- **Promotes to ADR:** no

### [5] Keep the local DB host fixed at `127.0.0.1` and say so

- **Decision:** `resolve_local_connection` sets `HOST="$PERSONAL_DB_HOST"` unconditionally, and `usage()` states that `--host` is ignored for a local deployment.
- **Alternatives:** Honor `--host` for local, or read `.connection.host`. Rejected: the local SQL endpoint is always a launcher-managed forwarder on the invoking host, so an override has no meaning.
- **Rationale:** The bug worth fixing is the silence, not the constant. Documenting the ignore removes the silence without adding behavior.
- **Promotes to ADR:** no

### [6] Rename `PERSONAL_DB_PORT` to `PERSONAL_DB_PORT_DEFAULT`

- **Decision:** Rename the constant; it stays as the fallback argument to `deployment_field` and the value `usage()` interpolates.
- **Alternatives:** Leave the name — rejected: `PERSONAL_DB_PORT` next to `PERSONAL_DB_HOST` reads as a fact about Personal, which is what the hardcode leaned on. Delete it and pass a literal `8563` — rejected: `usage()` needs the same value, and two literals drift.
- **Rationale:** Naming a constant for its role rather than its value makes the same mistake harder to repeat.
- **Promotes to ADR:** no

### [7] Correct the spec background and the user docs, not only the code

- **Decision:** The delta rewrites the background paragraph that asserts local Personal "publishes only the SQL port (`8563`)", adds the local resolution scenario, and amends two scenarios that name `8563` as the port ("Connection details are read fresh on every run", "A registered Rust UDF executes on Personal"). Task 4 applies the same correction to `docs/installation.md`.
- **Alternatives:** Spec-only correction, leaving `docs/installation.md` asserting a fixed `127.0.0.1:8563`. Rejected: it would leave the library and the user-facing docs contradicting each other on the field this plan exists to fix.
- **Rationale:** The false assertion is the root of the defect, so every place stating it is in scope. This is the documentation correction, not the deferred banner enhancement.
- **Promotes to ADR:** no

### [8] Defer the banner host:port print and the `deployment_ssh_port` deduplication

- **Decision:** Out of scope. `scripts/install.sh:467` and `:476` keep their current banners, and `deployment_ssh_port` keeps its hand-rolled read instead of calling `deployment_field`.
- **Alternatives:** Fold both into this plan. Rejected per the interview: the banner is an enhancement, and the deduplication touches a function with numeric validation that `deployment_field` does not provide, so it is a behavior question of its own.
- **Rationale:** Recorded so a reviewer reads their absence as a decision. Both are candidates for their own issues; printing the resolved `host:port` before `ALTER SYSTEM` would have made issue #83 self-diagnosing.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Task 1(c)'s `8563` assertion was vacuous (round 1 BLOCKER)

- **Finding:** `plan-reviewer` flagged task 1(c) `resolves_local_defaults_when_db_port_absent` as unfalsifiable. It asserted `PORT` is `8563` after `reset_connection_globals` had already written `PORT=8563` (`scripts/tests/install-personal-test.sh:126`), so the assertion passed whether or not `resolve_local_connection` applied the `deployment_field` default. The § Scenario Coverage row therefore claimed coverage for the default-fallback clause — the no-change path for every single-deployment operator — that no assertion could break. The model test `resolves_cloud_defaults_when_connection_fields_absent` (`:251-269`) is vacuous the same way, so copying it propagated the hole.
- **Direction change:** Task 1 now requires the sentinel `PORT="unresolved"` in all three tests, set after `reset_connection_globals` and re-set before every `resolve_local_connection` call, and states that (c)'s assertion is falsifiable only because of it. Tasks 1(a) and 1(b) carry the sentinel for uniformity: their expected values (`52164`, `59446`, `1234`) already differ from the reset value, so no assertion in the group depends on what `reset_connection_globals` writes. § Verification § Scenario Coverage now records that the default-fallback clause is covered only with the sentinel in place. Test scope is unchanged — three tests, same fixtures, same expected values.
- **Promotes to ADR:** no
