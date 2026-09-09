# Tool and delegated-worker identity

Checkpoint43 carries the active session's user, role and workspace through the actual tool/worker boundary. It does not select a new personal/shared-host policy.

## Reproduced behavior

Engram's automatic curator writes User-scoped facts, but ToolContext omitted the user identifier. EngramTool recalled/forgot with an empty user, and an explicitly requested user scope fell through to global. The real CLI fixture failed to return the caller's seeded private fact; investigation showed two independent source defects: dropped identity and destructive LIKE-query sanitization. Logs `implementation-engram-identity-before-smoke-v3.log` retain that failure. Initial fixture versions expected only4calls; observed5calls included a legitimate optional blueprint-author call, which the corrected fixture separately counts rather than hiding it.

JIT and automatic Hive workers used synthetic users and Admin. TemDOS used a synthetic core user/Admin and preferred process CWD over the caller's workspace. These construction paths could discard user identity or broaden a delegated role.

## Shared contract and production wiring

ToolContext is Clone and now includes user_id and role. `from_session` captures those fields, channel, workspace and existing read tracker from SessionContext. The production executor uses it; model arguments cannot select that identity. `delegated_session` retains user/role/workspace while creating private worker history, route and a fresh read tracker. Parent reads do not automatically satisfy a child's read-before-write gate.

JIT workers use the invocation context, including its actual workspace rather than the old composition hint. Automatic Hive captures the active parent session through the same helper; each task receives a private chat identifier. TemDOS InvokeCoreTool calls `run_scoped` with the caller context and no CWD preference. Core tool definitions are filtered by that role; executor enforcement remains authoritative even if a provider emits a forbidden call.

The existing standalone `CoreRuntime::run(task, workspace)` API remains a trusted-host compatibility path with its historical Admin behavior. Embedded hosts processing callers should use run_scoped. The outer runtime still blocks invoke_core for User; this checkpoint does not loosen that rule. Source users constructing ToolContext literals must provide user_id/role; application code should prefer from_session. This is part of the intended v6 source migration, not a published5.8.x API update.

## Engram behavior

An explicit user scope uses the authenticated caller, and requires a nonempty identity. Unknown scope strings return an error instead of silently selecting global. Recall/forget pass the actual user identifier and can now reach curator-created facts. SQLite now escapes LIKE metacharacters rather than removing them: underscores, percent signs and the escape character remain literal query content. A percent-only query no longer becomes a match-all expression. A focused pre-change SQLite test fails on key_1, and the unchanged CLI query also failed after the identity-only repair; both failures are retained. The legacy global default is preserved, as are existing stored records and scope-key conventions. There is no new implicit data migration. Historical synthetic-worker facts remain stored; their ownership is ambiguous, so they are not automatically reassigned to whichever user next invokes a worker.

## Acceptance and boundaries

The CLI fixture uses a real tool loop and SQLite reopen to inspect own/foreign facts, explicit user writes and deletion. It checks four foreground requests plus one separately identified optional blueprint call. It is independent of the model's final prose.

A real JIT execution test uses SQLite Hive coordination and two sequential tasks. It passes a deliberately wrong composition workspace, supplies a caller workspace with a marker file, and has the model attempt a forbidden shell call before a context probe. The shell executes zero times, both probes run under User in the caller workspace and retain the caller's user. The first fixture used one task and did not activate Hive; that failed test is retained, and the fixture now satisfies the existing two-task activation condition rather than changing production activation policy.

A direct InvokeCoreTool test exercises registry resolution, the core runtime and executor against the caller workspace. Even an embedded invocation with User context cannot run the forged shell call; the actual probe reads the caller's marker and sees the caller identity/role. Core helper tests cover both Admin and User inheritance and isolated history/read tracking.

This carries the existing role policy, not a complete capability system or OS sandbox. Role revocation during in-flight work, inherited arbitrary tool filters, cross-channel user namespace migration, global-memory write authority and durable delegation lineage remain open. Private worker routes are not delivery authorization. Engram's default no-op backend methods and ambiguous first-match deletion need the next persistence checkpoint; identity correction alone does not fix those. No automatic durable pursuit or shared-host default is introduced.

## Final validation

Validation: 792agent,262core(one ignored),345tools(default features),16cores,71memory(one ignored),79root and50TUI tests pass. Actual JIT SQLite coordination runs two tasks with caller identity/User/workspace; forged shell executes0times. Direct TemDOS invocation also reads the caller workspace and blocks shell. Real CLI passes4foreground+1optional-blueprint requests, recalls/deletes the caller fact, stores new user-scoped data and preserves/unexposes the foreign fact (`implementation-engram-identity-final-smoke.log`). Initial CLI counts omitted the blueprint call; corrected before analyzing identity. Identity-only repair still failed due destructive LIKE sanitization; a separate SQLite pre-change test reproduced literal-query failure, then escaped query repair passed. All failures remain in before/after-smoke and literal-before logs. Initial one-task JIT fixture did not activate Hive; corrected to the existing two-task requirement, with original failure retained. Full workspace/all-feature/all-target clippy passed39.87s and cleaned3.8GiB (`implementation-engram-identity-workspace-clippy.log`); dependency future-compatibility notice remains. No active Cargo or paid calls.
