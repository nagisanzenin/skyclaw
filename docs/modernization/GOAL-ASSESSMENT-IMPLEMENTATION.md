# Durable observations and declared-check assessment

Checkpoint 51 connects actual Witness results to the active execution goal ledger. A declared-check assessment is stored separately from overall goal achievement and from raw tool/artifact evidence.

## Contract and implementation

The additive `goal_assessments` table holds one immutable assessment for an admitted goal and its frozen criteria hash. The version-1 typed document contains model-proposed coverage (`unverified`), a recomputed declared outcome and individually hashed evaluator observations. Each observation records its kind (`evaluator_report`), schema, stable criterion ID (sealed Oath hash plus index), evaluator version, canonical workspace, observation time and exact predicate result. The full document is also hashed. Existing ToolResultEvidence and its resolver are unchanged.

Recording requires the exact original frozen Oath, execution/subtask/session, canonical workspace, authenticated user/role and admitted conversation epoch. Every returned predicate must match its sealed index, tier and required/advisory policy. Missing results, modified predicates, forged advisory flags or another subtask are rejected. Required/advisory status comes from the sealed predicate; the raw overall verdict does not control the stored assessment.

Core `assess_required` now has an active persistent consumer. It evaluates the declared criteria against resolved observation hashes and evaluator kinds/versions. A required deterministic failure dominates an advisory or reported overall pass. Missing checks, empty reasons or unavailable/model evidence cannot pass. Deterministic observations use evaluator version `temm1e-witness/tier0-v2`, identifying the three-valued logic repair in50. Changes to evaluator semantics must update that version.

Current Tier1/Tier2 wiring supplies only a workspace/subtask label and ignores evidence_refs. Its returned model report is preserved faithfully, but the stored grounded assessment remains Inconclusive for that required model check, with version `temm1e-witness/model-report-unresolved-v1`. A model's PASS is not silently recast as an observed artifact fact. Advisory checks remain advisory; they cannot erase a required failure.

The transaction advances an expected goal revision, inserts the unique assessment and appends `declared_checks_assessed` together. Both goal and execution must remain running. The active runtime expects revision1 after initial criteria, records assessment at revision2 and returns at revision3. A stale concurrent writer, duplicate insertion, terminal execution or binding mismatch rolls back. The runtime logs persistence failure and leaves goal achievement unverified; it does not invent a replacement assessment. Returning a reply with an assessment now says that declared checks were recorded and full coverage is unverified, instead of incorrectly saying no assessment ran.

Per-report detail is limited to16KiB; the serialized assessment to4MiB and the observation count to128. Oversized documents are rejected without claiming truncated evidence was fully preserved. Records are additive and no historical success is inferred. The scoped resolver validates document hashes, criterion hashes, observation hashes, schema and recomputed outcome before returning a result. Missing/foreign records disclose no other scope's contents.

## User inspection

`/goal-assessment <goal-id>` is a read-only common owner command available in CLI/server/TUI, with a TUI menu entry. Goal IDs come from `/goal-status`. It displays the recorded declared outcome and explicitly unverified full-request coverage, then up to12checks with160-character detail previews. Terminal control characters are escaped. Full typed reports remain saved. Inspection after restart makes no provider call. A missing assessment is reported as missing, never a pass.

A report hash proves which evaluator statement is retained relative to that hash; it does not supply raw file contents, test output or artifacts the evaluator did not snapshot. Reports may describe time-local state that later changes. Full request coverage remains unverified for the model-proposed set, including when all declared checks pass. No overall succeeded transition, automatic pursuit or privilege default is introduced.

## Validation

Before integration the actual runtime test failed because no goal_assessments table existed (`implementation-goal-assessment-before-test.log`). After integration the same real AgentRuntime/planner/Witness fixture persists three passed declared checks while retaining the original broader request and `awaiting_evidence`, at revision3.

Five storage/assessment tests cover required failure despite overall/advisory PASS, unresolved model evidence, missing/mismatched predicate/tier/advisory/subtask/authority, one concurrent winner, duplicate rollback, scoped reopen/tampering, terminal/oversized rejection and prior-criteria preservation. The first complete suite passes789agent unit+72integration (`implementation-goal-assessment-suite.log`). A sixth command test adds terminal-control escaping and bounded12check output; its final entrypoint-suite results are recorded below.

Three actual CLI fixture scenarios pass: all declared checks Passed; unknown composite Inconclusive; known required failure plus an unknown check Failed. Each unlimited scenario uses4requests/2planners and saves2assessments. Each capped control makes1planner/0foreground, retains its proposal and correctly reports no assessment. All document/observation hashes, original objectives and restart `/goal-status` + `/goal-assessment` inspection pass with zero additional provider calls. Logs are implementation-goal-assessment-cli-{passed,unknown,failed}.log. No paid service or real user profile is used. Binary build passed46.39s.

Final entrypoint suites pass 790 agent unit + 72 integration, 79 root + 14 integration and 50 TUI tests; 3 existing agent doctests remain ignored (implementation-goal-assessment-entry-tests.log). The command-control regression also passes. Full workspace/all-feature/all-target clippy passed 1m47s and cleaned 5.0GiB (implementation-goal-assessment-workspace-clippy.log). Existing dependency future-compatibility notice remains.

## Limits and next work

Evaluator exceptions currently leave no successful assessment record; their unavailable status is logged and the goal remains unverified. Persist typed unsuccessful attempts and resolve actual scoped immutable evidence into Tier1/Tier2. Bind those verifiers to the owning provider/model/budget, retain usage on failed/cancelled calls and surface missing evidence without paying for an ungrounded verifier. Do not treat recorded ungrounded model reports as completion evidence.

Next also investigate checkpoint48's Windows cancellation timeout separately from SQLite admission/final durability; checkpoint49 passing CI is not proof that48's failure was repaired. Preserve its failed log. Queue leases/reconciled continuation, delegated lineage, complete request coverage, total retention and external integrity anchoring remain open. D01/D02 defaults and the broader A/B/release gates remain unchanged.
