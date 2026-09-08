# Implementation packets

These are ordered implementation instructions, not a claim that fixes are implemented. Each packet should be a small series of PRs. Start from the pinned baseline plus this documentation branch. Read the associated finding before editing. Do not combine a behavioral repair with renaming unrelated code. New shared contracts belong in core; leaf implementations remain in their existing crates.

## Rules for the implementing model

For each packet: record baseline behavior; add the smallest failing acceptance fixture; implement; run the listed focused crate tests and relevant real-entrypoint fixture; inspect the actual outbound request/persisted record; report results and remaining limits. A passing mock-only test cannot close a production-wiring criterion. Never report a skipped stage as Passed, fabricate usage, delete failing assertions, or substitute prose for expected artifacts. Use fake provider/local servers for deterministic tests; live account tests are separate and must record endpoint/model/date. Creator-gated choices in DECISIONS remain gated.

## P01 — Immediate correctness repairs

Scope `codex-oauth/responses_provider.rs`, `agent/runtime.rs`, `perpetuum/self_work.rs`; findings F01/F03/F06/F32. No architectural dependency.

1. Build a request with base and volatile sentinels; assert both appear exactly once in serialized Codex instructions. Use shared flattening semantics; do not move volatile state into a permanently cached base.
2. Replace maintenance false success with `NotImplemented`/`Skipped` including reason until real work exists. Persist the actual outcome and prevent success reinforcement.
3. Remove the outgoing empty DONE reminder from the active reply path after reproducing it. Keep public legacy type available until P03 replaces it; this fix alone does not implement verification.
4. Extend usage parsing through P10/P14 before cache savings claims; add Anthropic cached-read and write fixtures immediately.

Acceptance: wire body contains both sentinels; no-op maintenance cannot increment completed-work counters; final reply does not contain an empty post-inference instruction. Run oauth/perpetuum/agent library tests. Revert by ordinary code rollback; no storage migration.

## P02 — Durable goals, recovery and cancellation

Scope agent task_queue/recovery/runtime, core types, main and TUI bridge. Depends P01; F02/F23.

1. Define states/events as in architecture, with schema version and principal/workspace. Add migrations using transactions and backup; preserve legacy conversation data.
2. Implement enqueue and transactional lease claim with compare-and-swap generation. Unique operation IDs and unique delivery keys prevent duplicate ledger writes.
3. Checkpoint before and after each tool: intent, arguments hash, idempotency key, outcome/evidence. If process dies between intent and outcome, mark UnknownOutcome; reconcile external state before replay.
4. On startup recover expired leases and unfinished operations. Do not blindly replay shell/browser writes. Differentiate resumable reads from uncertain effects.
5. Wire shared service into Start, Chat, TUI, model switches, Hive/Cores and Perpetuum. Replace per-path ad hoc history completion with events; leave historical records readable.
6. One cancellation token reaches provider streams and tool subprocesses. Stop accepting new work on SIGTERM/Ctrl-C; checkpoint and drain to configured deadline; preserve remainder for recovery.

Acceptance: launch real daemon with fake provider, kill after intent and after tool return, restart and verify one side effect and continued goal; concurrent lease claims yield one owner; stale worker cannot complete; outbox resend deduplicates. Tests must exercise factory/entrypoint, not only TaskQueue. Rollback preserves ledger and stops new writes before schema downgrade.

## P03 — Evidence-backed completion

Scope witness_init, Witness evidence resolution, runtime, watchdog; depends P02; F03–F05/F17.

1. `Criterion { id, description, evaluator_kind, required, evidence_refs }`; `Assessment { Passed, Failed, Inconclusive, evaluator_version, evidence_hashes, reason }`. Model proposes criteria; preserve explicit user requirements mechanically.
2. Construct Witness per execution workspace and wire configured verifier tiers through the resolved provider. A disabled tier is visible; absent evidence is Inconclusive.
3. Resolve artifacts/tool exits/test reports into bounded immutable evidence snapshots. Include content/hash/source/time and whether the evaluator actually inspected it. A path string is not evidence of file contents.
4. Keep deterministic checks authoritative for their narrow claims. An LLM audit cannot override a failing mechanical requirement or claim tests ran without event evidence.
5. Replace self-audit cleanup with typed audit results; retain substantive corrective feedback. Separate delivery from completion. Implement creator-selected Witness Law5 behavior only after decision.
6. Hash-chain ledgers detect tampering relative to a trusted anchor; document same-user process limits. External trust requires a separate identity/service, not chmod alone.

Acceptance: wrong workspace fails; missing verifier is visible; build passes but task test fails => not succeeded; evaluator exception => inconclusive; corrective audit survives; partial reply can send without falsely completed goal. Test config through main/TUI factories.

## P04 — Final context compiler

Scope context.rs, message types and all provider codecs; depends model resolution P05; F07/F32.

1. Gather base, volatile, goals, evidence, tools, images, history and output reserve before budgeting. No downstream mutator may append unaccounted content.
2. Compute token count with model-aware adapter; expose estimated vs exact. Reserve protocol and next-output capacity; reject impossible fixed content with a recoverable error.
3. Group complete tool turns. Move oversized output into scoped artifacts with hashes and excerpts. Preserve current goal and latest steering outside eviction candidates.
4. Run P14 compaction when needed, then enforce the invariant again on serialized request. On provider overflow, use one bounded recompaction/recount path; never drop random messages and retry indefinitely.
5. Record allocation by category and estimator error against returned usage, accounting for cache semantics.

Acceptance: enormous schema, late profile injection, image-heavy request, CJK, custom small-context model, oversized fixed goal, tool-pair boundary. Every sent request fits under fixture tokenizer; impossible request is not sent. Maintain existing λ/Engram/Blueprint allocation as policy inputs, not independent capacity authorities.

## P05 — Resolved models and credentials

Scope core model_registry/custom_models/config, provider factories, OAuth, main/TUI. F01/F10–F12. Full schema in subscriptions document.

1. Key models by provider + endpoint kind + model ID + revision; capability values carry source/date and Unknown.
2. Resolve one immutable snapshot per turn including context/output, modalities, tools, reasoning parameters, cache/compaction modes and account entitlement. Unknown model requires explicit capabilities or conservative diagnostic, not automatic vision=true.
3. Replace substring pricing and scattered lookups at every caller. Custom models pass through the same resolver. Refresh catalog separately from active turn to avoid mid-request changes.
4. Implement process-safe refresh/atomic credential writes and redacted Debug. Keep old credential migration read-only until validated new write.
5. Offer distinct account and API-key connections; validate selected model on that connection with a minimal permitted capability request. Authenticated is not entitled.

Acceptance: custom provider name collisions; account lacks chosen model; catalog unavailable uses labeled last-known snapshot; two processes refresh; crash during save; expired/revoked login; no secrets in logs. API/subscription model sets can differ.

## P06 — Subscription lifecycle and quota

Scope account service, OAuth adapters, slash commands/TUI. Depends P05/P10.

Implement exactly the state machine and fixtures in 06-SUBSCRIPTIONS. Add headless/device flow where officially supported; cancellation and expiration; quota windows; explicit API fallback choice. Unsupported third-party subscription use stays unavailable with reason. Do not hardcode borrowed client IDs or scrape another application's credentials as the onboarding design. Migration preserves current working login until replacement is proven.

## P07 — Tool execution and side-effect integrity

Scope executor, tools/shell/code_*/browser/desktop, MCP. Depends shared context P02; F08/F09/F26/F27/F31.

1. Add trusted `EffectSpec` with reads, writes, external effects, resource keys, idempotence and cancellation behavior. Default unknown tools exclusive. Shell is exclusive absent an explicit trusted constrained adapter; name/path guesses never establish independence.
2. Launch subprocesses under owned process group/job object; stream stdout/stderr into bounded buffers/spool; timeout kills and reaps descendants. Return cancellation acknowledgement only after cleanup or explicit unknown state.
3. Serialize conflicting browser sessions, workspace paths and external resources; allow bounded proven-independent reads. Preserve original tool call order in results regardless of finish order.
4. Use content hashes in read-before-write checks. Multi-file patch preflights all changes, journals intended operations, writes temp files, commits/recovers deterministically. Snapshot uses temporary Git index and path manifest; preserve staging bytes and unrelated files.
5. MCP: supported-version negotiation, tools pagination with cycle/page/byte limits, schema validation and typed result parts; refresh tool catalog by revision. Transport cancellation and disconnect have explicit incomplete outcomes.
6. UI/browser action success requires task-specific postcondition; visual diff is supplementary. Never retry uncertain purchase/send/upload solely because pixels are unchanged.

Acceptance: baseline late shell marker must never appear after timeout; child/grandchild killed; huge output bounded; same-resource commands serialized; independent reads concurrent; staged changes unchanged by snapshot; restore handles extra files per declared scope; paginated tools complete; image/structured results retain type.

## P08 — Memory, identity and capability scope

Scope memory trait/backends, λ/learning/Engram, vault, RBAC/delegation; F13/F30. Depends P02/P05.

1. Make principal/workspace scope mandatory in new query APIs, including candidate/FTS searches. Shared knowledge requires explicit scope, never omitted filter.
2. Backfill unambiguous single-owner records after backup; quarantine ambiguous records. Test both search paths and delete/export.
3. Pass parent principal/workspace to cores and Hive; child capabilities are a subset. Remove CWD preference. Personal owner bootstrap and shared-host defaults follow creator decision.
4. Capability gates apply to newly registered MCP/custom tools by effect, not four-name blacklist alone. Keep personal-host autonomy configured explicitly.
5. Vault writes use private creation permissions, atomic rename and process-safe locking; distinguish local at-rest encryption from protection against same-user host access. Add retention/GC metrics for all persistent buffers.

Acceptance: two users with identical queries cannot retrieve each other's λ/knowledge; core sees caller workspace; added dangerous tool doesn't bypass User restrictions; corrupted role file does not elevate in shared mode; memory correction/deletion persists across restart.

## P09 — Cambium and learning promotion

Scope cambium pipeline/deploy/trust, runtime; F14/F17.

1. Declare exact gate set per growth class. `Skipped(reason)` can only be acceptable for an explicitly optional gate; required unimplemented gate blocks deployment.
2. Implement real brief, zone checks, build/lint/format/tests, review/security/integration hooks. Evidence identifies actual command/artifacts. Compile success proves compilation only.
3. Generate in isolated staging/worktree; diff against permitted paths; preserve host work. Trust promotion uses verified growth outcomes scoped to zone, not general user task successes.
4. Deploy through atomic version switch with health check and rollback. Keep previous artifact, migration compatibility and watchdog ownership explicit.
5. Retain default behavior until creator approves any change in autonomous shipping scope; improve truthful status immediately.

Acceptance: compiles but breaks integration is blocked; protected path modification rejected; missing reviewer shows incomplete; deployment health failure rolls back; process dies mid-switch recovers one version. Do not auto-promote from a streak without recording what was verified.

## P10 — Global resource admission and perpetual operation

Scope budget, all LLM call sites, Perpetuum; F06/F15/F25/F28/F32.

1. Replace String-only cognitive call result with content+normalized usage+request ID. Route consciousness, Anima, Witness, classifier, Hive, Cores, Eigen and maintenance through shared admission.
2. Reserve bounded maximum cost/tokens before concurrent calls; reconcile actual usage and release on known failure. Persist ambiguous reservations for reconciliation, do not silently zero them.
3. Maintain separate API spend, subscription quota, local compute and per-goal limits. Exhaustion returns PausedQuota/BlockedBudget with reset/evidence; no silent model/account switch.
4. Activity estimator counts exposure slots and positive activity separately with real calendar cutoff. Scope user/timezone; missing observations stay unknown. Define cron DST and outage catch-up policy explicitly.
5. Implement maintenance handlers or report skipped. Every growth buffer declares record/byte/time cap, cleanup job and observable last-cleanup result. Retry respects cancellation and account not-before deadlines.

Acceptance: concurrent calls cannot all pass the same remaining budget reservation; background calls counted; cache categories reconcile; one active of28 observed slots not1; offline slots not labeled inactive; process restart retains concerns and spend policy; cleanup actually removes seeded expired records.

## P11 — Statistics and evaluation

Scope distill stats/curator/judges and all lab claims. F16/F19/F24/F29; see math audit.

1. Validate finite/domain inputs in statistical constructors. Separate inverse normal quantiles from confidence levels.
2. At SPRT cap return Inconclusive unless a calibrated finite-horizon rule is implemented. Label behavior/embedding signals proxies. Preserve double opt-in for local serving.
3. Group splits by source task/session and time before training; near-duplicate filtering complements exact hash dedup. Fix category coverage independently of entropy and quality sampling.
4. Add deterministic numerical fixtures and seeded error-rate simulations; publish parameters and stopping rules. Calibrate CUSUM false alarm/delay on representative sequences.
5. Build evaluation corpus by feature and failure mechanism; run paired same-model comparisons at equal budgets. Measure success, unsupported-completion rate, wall latency, all-call cost, recovery and retention. Report sample sizes and uncertainty; no extrapolation from 30 tasks to universal reliability.

Acceptance: known quantiles and Wilson examples; cap below boundary never graduates; missing categories fail coverage; contradictory but similar answers fail task evaluator; no training pair appears in held-out group. Documentation claims link to raw evidence.

## P12 — Composition refactor

Scope main.rs, runtime builders, TUI bridge. Depends tests from P01–P05; F18/F30.

1. Inventory every runtime constructor/rebuild; give each an entrypoint ID. Create `RuntimeFactory::build(ResolvedConfig, SharedServices, ExecutionContext)`.
2. Move one path at a time with serialized effective-config comparison. Test Witness tier wiring, task ledger, budgets and model snapshot through each.
3. Split command parsing, credential lifecycle, channel setup and process supervision into modules. Do not create an enormous replacement factory with hidden global state.
4. Delete duplicated chains only after parity fixtures pass. Deprecate unused legacy managers with feature replacement mapping; preserve exported APIs for migration.

Acceptance: Start/Chat/TUI/model-switch instantiate identical requested services; no independent Admin/CWD/model-default substitution; dependency graph remains acyclic and leaf boundaries intentional.

## P13 — Channels, cloud scaffolding, observability and distribution

Scope Slack/other channels, filestore, observable, gateway, tenant/orchestrator, installer; F20–F23/F28/F35.

1. Slack external upload sequence with local HTTP fixtures; persistent pagination/dedup and rate-aware ingestion. Other channels require disconnect/reconnect, duplicate delivery, file size and formatting fixtures; live platform tests separate.
2. S3 abort incomplete uploads on failure and persist cleanup tasks for crash; bounded chunking and verified completion. Test empty/final-small-part and failure at each multipart step against compatible local service.
3. Real OTLP exporter with bounded batches/histograms and shutdown flush; health from last export attempt. Wire into composition and verify collector receives trace IDs across request/tool/goal.
4. Mark cloud identity/tenancy/orchestration experimental until implemented. Add real Docker client only behind integration tests; capacity reservations transactional, provisioning retries idempotent and orphan cleanup. Kubernetes remains unsupported until actual adapter exists.
5. Shared dashboard endpoints inherit authentication/capability policy; redact secrets and use observed component status, not configuration presence. Test unauthorized reads and stale health.
6. Installer/updater verifies exact asset hash and authenticity policy; missing checksum cannot be reported verified. Define an explicit development override instead of implicit skip; package smoke across target triples and rollback after interrupted update.

Acceptance: no claimed-success stub, Slack pagination burst retained, export failure visible, multipart cleanup, tenant boundary test, concurrent capacity bound, missing checksum stops normal install. Do not deploy cloud services as part of this research branch.

## P14 — Compaction and caching

Full algorithms, schemas, failure modes and acceptance fixtures are in [07-CONTEXT-CACHING](07-CONTEXT-CACHING.md). Implement final context accounting and cache usage normalization before optimization. Preserve raw events and active constraints across compactions; model-native opaque state stays provider-specific.

## P15 — TUI

Creator selected compact transcript with expandable tools and optional panels. Implement the ten steps and viewport/event fixtures in [08-TUI](08-TUI.md). Preserve palette and keyboard workflows. The UI consumes actual execution/account/context events; it must not synthesize success from a spinner ending.

## Suggested delivery tranches

A: P01 plus F08 shell cleanup and cache accounting fixtures. B: P02–P05/P10 shared contracts, P12 factory and P14 continuity. C: P06/P07/P08 and P15 end-user workflows. D: P09/P11 calibrated learning and P13 production cloud capabilities. Tests from P11 run throughout. Each tranche ships independently only after its acceptance criteria; estimates require implementer sizing, not invented day counts.
