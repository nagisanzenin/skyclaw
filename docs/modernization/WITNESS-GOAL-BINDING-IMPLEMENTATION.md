# Witness goal binding and planner accounting

Checkpoint47 repairs the inputs to persistent criterion integration. It preserves the user's actual request in a sealed Oath, gives each execution its own verification identity and charges planner calls to their owning runtime.

The planner may propose postconditions but cannot replace the goal text. `seal_oath_via_planner` now overwrites the model draft's goal with the exact user request before Spec Reviewer validation and sealing. Consequently a model cannot evade the existing code-task rigor check by renaming the task as a simpler objective. The lower-level explicit `oath_from_draft` host helper remains unchanged for compatibility. This does not prove that model-proposed postconditions cover every part of the preserved request; weak coverage remains an explicit gap.

Runtime Oath root IDs use the admitted durable execution ID. Non-durable embedded calls use a fresh per-call UUID. Subtask IDs derive from that same identity. Conversation session IDs are retained for retrieval but are not reused as goal/subtask identity across turns. Existing ledger entries are preserved, not re-keyed or retroactively assigned to executions.

Planner calls use the existing shared MeteredProvider. Successful response usage is retained before JSON parsing or Spec Reviewer errors, and provider error/cancellation records one unknown outcome. The selected provider/model and existing planner activation/strictness/output policies remain unchanged. There is no automatic continuation or new completion/delivery rule.

## Evidence

Three pre-change actual planner/runtime tests reproduced the defects: the sealed goal became the model's weaker replacement; two turns shared one root/subtask ID; only2of4provider calls were charged because planner usage was discarded. Their unchanged repaired versions pass. An additional test covers malformed JSON, provider error and cancellation with exact known/unknown accounting and no double charge. All fixtures use an isolated actual SQLite Witness ledger and local mock providers.

The first before-test compilation failed an ambiguous Path::into type in the fixture. It was corrected to an explicit PathBuf before observing all three real failures. Logs: `implementation-witness-binding-before-tests.log`(fixture compile), `-before-tests-v2.log`(three actual failures), `-after-tests.log`(three pass), `-final-tests.log`(four pass). Broader checks pass780agent unit+68agent integration and94Witness unit+33Witness integration tests. Real CLI validates original goals and distinct execution-bound Oaths across two turns(4requests/2planners); the capped control makes1planner request and0foreground calls after that charge. This is a between-call admission gate, not a concurrent reservation or zero-overshoot guarantee. Actual narrow fixture predicates test wiring, not coverage of arbitrary user intent. Build passed45.90s, and full workspace/all-feature/all-target clippy passed1m49s, cleaning3.4GiB. Logs: `implementation-witness-binding-suite.log`, `-cli-smoke.log`, `-workspace-clippy.log`. Existing minimal-feature root and dependency future-compatibility notices remain.

## Remaining integration

Connect these execution-bound Oaths to scoped immutable criterion-set revisions and assessment/evidence observations in the active goal ledger. Preserve model-proposed versus explicit user requirements and unknown coverage. A Witness verdict string or narrow grep/build check cannot establish arbitrary task completion. Goal46 currently remains awaiting_evidence after reply even when a separate Witness verdict passes; persistent criterion assessment and succeeded-state policy are not implemented by this binding repair.

Tier1/2 evidence resolution, configured verifier/provider resource binding, truthful per-verifier costs, guarded evaluator side effects, child lineage and queue recovery remain separate work. The current code-shaped planner gate and legacy Spec Reviewer keyword gate are still heuristics; retaining exact objective text prevents a specific bypass, not all weak criteria. D01/D02defaults remain unchanged.

Source review found that automatic command predicates currently lack the caller role gate, despite workspace binding. The next checkpoint prioritizes an actual negative/positive authority fixture and repair before criterion integration or release. No exploit test has yet run at this checkpoint.
