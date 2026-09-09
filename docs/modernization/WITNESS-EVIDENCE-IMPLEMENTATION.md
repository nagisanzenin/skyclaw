# Grounded file evidence for Witness

Checkpoint 53 supplies actual scoped file contents to model verifiers and preserves those exact contents in the active goal assessment. It replaces the old workspace/subtask label that was presented as evidence. Full request coverage remains explicitly unverified.

## Source and data contracts

PlannerOathDraft accepts an optional evidence_required list of typed EvidenceSpec declarations. Each model predicate names evidence_refs that must resolve uniquely inside that sealed Oath. Old JSON drafts without the list still parse as an empty list; Rust struct-literal callers must add evidence_required: vec![]. The planner prompt describes this optional contract and does not require inventing a semantic review just to use a model verifier.

Only regular UTF-8 file sources are currently resolved. Each file is limited to16KiB, each verifier to8references and32KiB of file contents, with a64KiB serialized-evidence bound. Goal/rubric byte bounds are16KiB/8KiB. Duplicate, missing, oversized, binary, directory and unsupported sources abstain before a model call. Commands, HTTP requests and other effects are not executed merely to manufacture evidence; those source kinds need an execution-event resolver.

Paths are canonicalized and constrained to the actual workspace. Metadata is checked before and after bounded reading; detected size/mtime changes reject capture. Unix opens also use O_NOFOLLOW/O_NONBLOCK to reject a replaced leaf symlink and avoid blocking on a FIFO. The already-locked libc dependency is added for Unix flags and a controlled FIFO test. These checks do not provide an atomic filesystem snapshot or defeat every malicious ancestor-path race; they are not an OS sandbox or a guarantee about unresponsive network filesystems.

A FileEvidenceSnapshot records schema1, source ID, original declared path, resolved relative path, canonical workspace, goal/subtask/session/Oath binding, capture time, complete bounded UTF-8 content, byte count and SHA256. Offline validation uses those saved bytes, even if the source later changes or disappears. Metadata-only EvidenceProduced rows and their previews are not substituted for missing contents.

## Dispatcher and persistence

verify_oath_report returns the verdict plus per-predicate snapshots actually supplied to configured model verifiers. The legacy verify_oath API delegates to it and returns the verdict only; embedded hosts needing replayable bytes must retain the report. The active AgentRuntime uses the report and persists its artifacts in GoalAssessment, avoiding a second generic blob store in the Witness database.

Missing/disabled tiers and unresolved sources make zero verifier calls; tier call counters now reflect attempts actually made. Provider error reports remain Inconclusive with captured inputs retained when supplied. Verifier prompts permit pass/fail/inconclusive, require abstention for insufficient evidence and treat file contents as untrusted data. A response with no explanation is also Inconclusive. Tier2 remains advisory, and known deterministic failures remain authoritative.

GoalAssessment adds a serde-defaulted model_evidence field, keeping prior version1 records readable. Its scoped resolver checks each source against the exact Oath, expected predicate index/reference order, workspace, content hash and bounds. Grounded model observations use evaluator version temm1e-witness/model-file-evidence-v1. Older reports without captured inputs retain model-report-unresolved-v1 and remain Inconclusive as grounded assessments. Captured bytes support a model judgment about those bytes, not deterministic proof that tests ran or that the whole request was completed.

## Validation

Three before-tests reproduced the actual dispatcher failures: missing references still reached a model and passed; a real file's marker was absent from verifier input; outside-workspace/unsupported declarations still passed. See implementation-witness-evidence-before-tests.log. The unchanged tests pass after repair.

Seven evidence integrations cover actual input bytes, missing/ambiguous/repeated references, per-file/aggregate bounds, invalid UTF-8, source changes/removal, Oath/content tampering, outside symlinks and FIFO rejection. A real AgentRuntime/planner/custom-verifier/SQLite integration confirms file bytes arrive at the verifier, survive source replacement and reopen, stay scoped, and fail integrity checks when an inner blob is changed even if the outer document hash is updated. All declared checks can pass while the original broader goal remains AwaitingEvidence.

The first combined suite failed4old Witness fixtures: positive model checks supplied no evidence, and an absent auditor was counted as1call. These failures are retained in implementation-witness-evidence-suite.log. Positive/advisory fixtures now use actual file declarations; absent tiers count0attempts. The v2 Witness suite and final combined suite pass790agent unit+76integration and94Witness unit+45integration tests. A version1 legacy-report fixture confirms omitted model_evidence does not retroactively ground an old model PASS. Logs: implementation-witness-evidence-suite-v2.log and implementation-witness-evidence-final-tests.log.

Actual CLI regressions pass ordinary declared checks, a known failure plus unknown composite, and an explicitly disabled model tier with a parsed file declaration. Each unlimited scenario makes4requests/2planners; each capped control makes1planner/0foreground. Hashes, original goals and restart inspection with0extra calls remain correct. These are disabled-tier/main-entrypoint controls; the positive grounded-model case is the actual AgentRuntime integration with an explicit verifier attachment. No live paid provider or user profile was used. Binary build passed47.52s. Full workspace/all-feature/all-target clippy passed1m46s and cleaned3.6GiB (implementation-witness-evidence-workspace-clippy.log); existing dependency future-compatibility notice remains.

## Remaining factory/accounting work

The main attachment factory still does not apply configured Tier1/Tier2 provider flags or max_overhead_pct. This checkpoint does not claim that enabling those config flags now produces a live model verifier. Next wire current RuntimeResources and owning metering into each turn, including model switches/delegation, with bounded calls/input/output and explicit overhead policy. Do not silently enable new paid work without an enforceable policy. Subscription quota is not a zero-dollar token price, and unknown USD cost cannot be reported as satisfying a percentage cap.

Model-specific final wire-token fitting, per-attempt/global reservations and truthful Witness cost readouts also remain open; the existing numeric verdict cost field is not an invoice ledger. Persist typed unsuccessful verification attempts and add execution-bound command/test/HTTP evidence sources. Queue leases/lineage, complete requirement coverage, retention and external trust anchoring remain separate work. D01/D02 defaults and broad A/B/release gates are unchanged.
