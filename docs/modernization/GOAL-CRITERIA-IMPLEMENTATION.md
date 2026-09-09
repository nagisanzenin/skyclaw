# Frozen criteria in the active goal ledger

Checkpoint 49 saves the actual planner Oath into its admitted execution's goal ledger before foreground work can use it. This closes a persistence/composition gap; it does not claim that a model's proposed checks fully express the creator's request.

## Contract

The additive `goal_criteria` table permits one immutable initial proposal per goal. Its typed document has schema version 1, `origin: model_proposed`, `coverage: unverified`, the canonical workspace and the complete sealed Oath. Original goal text, predicates, evidence specifications, template information and Oath hash remain available together. These are proposed criteria, not observed artifact evidence. Existing `ToolResultEvidence` storage and its typed resolver are unchanged.

`record_goal_criteria` is a crate-private runtime operation. It requires the exact admitted execution ID, original objective, session identity, canonical execution scope and authenticated user/role snapshot. An admitted conversation epoch must match. The sealed Oath hash must recompute correctly; empty postcondition sets, more than 128 preconditions or postconditions, and documents above 2 MiB are rejected without truncating the original objective. These bounds are not a proof that the predicates are sufficiently strong.

The write uses an expected goal revision and requires the execution and goal to remain running. Revision advance, unique snapshot insert and ordered `model_criteria_frozen` event are one SQLite transaction. A concurrent loser, duplicate proposal, terminal execution or binding mismatch leaves the prior document and revision untouched. There is no replacement API: a later model cannot quietly overwrite this initial proposal or the user's objective.

AgentRuntime freezes the proposal immediately after planner sealing, before adopting it for the turn. A durable binding failure is logged and the turn continues unverified, with no use of that unbound Oath for a verification claim. Non-durable embedded runtimes keep their existing explicit host behavior. The separate Witness ledger may retain a sealed Oath if durable binding fails; it is not treated as active goal evidence.

The scoped `goal_criteria` resolver checks the conversation access domain before returning data, validates document and Oath hashes, schema and original objective. It does not need a provider call. `/goal-status` reports whether a model proposal was saved and explicitly labels coverage unverified. The immutable raw snapshot can be inspected through the typed host API; this checkpoint does not add an unbounded terminal JSON dump.

The schema change is additive: legacy execution records are not backfilled with invented criteria. The existing goal schema remains compatible; this new document carries its own version. No full database copy or user-profile migration is performed.

## Validation

Before implementation, an actual AgentRuntime/planner/Witness run returned a reply but inspection failed because the goal had no persistent criteria table (`implementation-goal-criteria-before-test.log`). The same integration now confirms original objective, execution binding, model-proposed/unverified status, revision 2 after return and `awaiting_evidence` despite DONE prose.

Four storage tests cover scoped reopen and tampering, duplicate insertion rollback, changed original objective/user/role/workspace/epoch/goal/seal, one concurrent winner, terminal rejection, empty/oversized proposals and original-goal preservation. All 784 agent unit and 72 integration tests pass (`implementation-goal-criteria-suite.log`). The first storage-test build had two fixture type errors (Option reply and PathBuf); these were corrected and the failed log is retained (`implementation-goal-criteria-unit-tests.log`).

Actual CLI validates snapshot persistence before the foreground HTTP request, exact hashes/workspace/objectives, unlimited 4 requests/2 Oaths, capped 1 planner/0 foreground and restart inspection with zero provider calls (implementation-goal-criteria-cli-smoke.log). Binary build passed 45.58s. Full workspace/all-feature/all-target clippy passed 1m49s and cleaned 3.1GiB (implementation-goal-criteria-workspace-clippy.log). Existing dependency future-compatibility notice remains.

## Next work and limits

First correct the newly inspected three-valued composite logic: AllOf/AnyOf currently collapse unknown into failure, which NotOf can turn into a false pass; empty/all-advisory aggregate can also pass. Reproduce and repair before using those outcomes as assessments.

Persist the actual assessment observations against this exact frozen set, with evaluator kind/version, required/advisory distinctions and resolved evidence. Missing or disabled evaluators stay inconclusive; required failures cannot be hidden by an advisory/higher-tier pass. A verdict detail string does not supply omitted raw artifact evidence. Track declared-criteria outcome separately from unverified coverage of the full request. Do not infer overall achievement from a weak model-proposed build/grep check.

This checkpoint does not implement assessment persistence, an overall succeeded transition, leases/reconciled queue continuation, child lineage, total database retention or external integrity anchoring. It adds no provider call, paid service or automatic pursuit. Existing D01/D02 privilege and continuation defaults stay unchanged.
