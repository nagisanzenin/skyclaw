# Feature coverage and preservation ledger

Every original FEATURES.md entry is mapped below, followed by later feature families and the complete source/document inventory. Coverage means design/source audit and a defined acceptance path; it does not mean every native platform, external account, integration or mathematical claim was experimentally validated. The baseline workspace library tests ran, but binary/integration/feature-flag tests and live services have separate limits in VALIDATION. No feature is declared healthy merely because no defect was found.

## Implementation evidence through checkpoint46

The tables below preserve the initial baseline audit. They are not the current implementation status. Use the linked implementation documents and checkpoint log for repairs and remaining limits:

| Feature families | Implemented evidence | Remaining acceptance boundary |
|---|---|---|
| Provider protocols, subscriptions and model pricing | `MODEL-CATALOG-IMPLEMENTATION.md`, `RESPONSES-IMPLEMENTATION.md`, `ANTHROPIC-NATIVE-IMPLEMENTATION.md`, `GEMINI-NATIVE-IMPLEMENTATION.md`; modern menu IDs, native observer integration and sourced image knownness through checkpoint 34 | Other capability knownness/endpoint-aware immutable model resolution, Google Interactions and live accounts other than authorized Z.ai remain separate. |
| Context, compaction, recall and cache metadata | `CONTEXT-IMPLEMENTATION.md`; guarded final fit, provenance/source retention and knownness-aware usage | Semantic retention breadth, auxiliary-call budgets and provider cache acceptance still require broader tests. |
| Conversation recovery, final delivery and TUI history | `SESSION-RECOVERY-IMPLEMENTATION.md`, `DELIVERY-IMPLEMENTATION.md`, `SESSION-IMPLEMENTATION.md`; checkpoints 18–19 and 28 | A committed transcript is not a completed durable goal; final outbox coverage does not include every interim/control/platform receipt. |
| Shutdown, routing, steering and background ownership | Checkpoints 17, 20 and 25 in `IMPLEMENTATION-STATUS.md`; actual CLI/server lifecycle and typed Mission Control tests | Crash-durable intake, shared goal/effect/budget composition and real channel reconnect remain broader work. |
| TUI presentation and input | Checkpoints 24 and 28; real Unix PTY, Unicode, resize, terminal restoration and zero-provider restart | Windows PTY and complete usability/large-history acceptance remain separate. |
| Installer/update/container/migration | `UPDATE-DEPLOYMENT-IMPLEMENTATION.md`; checkpoints 23 and 27; fully green CI 34256450049 including Docker replacement | Final-release update/migration and target artifacts require validation on the release revision. |
| Numerical reliability, identity and editing | Earlier math/reference checks, RBAC bootstrap, caller workspace propagation and temporary-index snapshot checkpoints | Global principal isolation, learning calibration and crash-atomic multi-file edits are not proved by these repairs. |
| Prowl blueprints | Checkpoint 27 parses every built-in with the real production schema and corrects unsupported login instructions | `BROWSER-IMPLEMENTATION.md` records owned profiles and tested origin-bound submission; no claim of 100+ verified live login services. |
| Engram persistence, identity and numeric policy | `ENGRAM-NUMERIC-IMPLEMENTATION.md`, `TOOL-IDENTITY-IMPLEMENTATION.md`, `ENGRAM-PERSISTENCE-IMPLEMENTATION.md`; checkpoints42–44, real scoped SQLite persistence/deletion and Markdown unsupported acceptance | EMA/provenance/cadence, channel-qualified principals and promised Markdown Engram fallback remain open. |
| Durable goals and evidence | `GOAL-LEDGER-IMPLEMENTATION.md`; checkpoint46 active admission/tool/return journal, scoped readonly CLI/TUI and actual persistence/hash/restart tests | Persistent criteria/Witness assessment consumer, task queue leases/recovery, child lineage and D02 continuation remain open; no success-from-prose. |
| Runtime resources and delegated model identity | `RUNTIME-RESOURCES-IMPLEMENTATION.md`; checkpoint45 immutable bindings, two-endpoint real CLI model switch/core and interleaved core/JIT tests | Full composition factory, Consciousness/Perpetuum lifecycle identity and durable global reservations remain open. |
| Hive scheduling and composed accounting | `HIVE-IMPLEMENTATION.md`, `SHARED-ACCOUNTING-IMPLEMENTATION.md`; stale publication/transactional completion and initial shared budgets, checkpoints 32–33 | Active-task recovery/generation ownership, durable budget reservations, alternate entrypoint and all auxiliary-call coverage remain open. |

## Original feature roadmap

Vision source: `FEATURES.md` and `docs/TEMM1E_VISION.md`; newer `VISION.md` can supersede older routing/isolation defaults. Source locations below are crate-relative unless main.rs.

| ID | Idea to preserve | Implementation inspected/mapped | Assessment, better implementation and acceptance |
|---|---|---|---|
| 0.1 | Graceful shutdown | main.rs daemon shutdown | F23: bounded joins exist; missing promised task drain/checkpoint. P02 supervisor and SIGTERM recovery fixture. |
| 0.2 | Provider circuit breaker | agent/circuit_breaker.rs; providers/rate_limit.rs | Keep breaker state/backoff; jitter and account quota distinct (F28). Test half-open concurrency and outage recovery, P10. |
| 0.3 | Channel reconnection | channels telegram/discord/slack/whatsapp implementations | Adapters exist; platform sessions need live reconnect proof. Durable ingestion watermark/dedup/outbox across reconnection, P13. |
| 0.4 | Streaming responses | providers stream implementations; agent/streaming.rs; tui/agent_bridge.rs | Provider support is not end-to-end streaming; F34. Propagate typed deltas/usage/cancel through runtime, P15. |
| 0.5 | Raised limits | core/types/config.rs; agent/runtime.rs | Larger round/time caps are policy, not robustness. Shared admission/cancellation and bounded outputs, P07/P10. |
| 1.1 | Verification engine | agent/executor.rs; witness | Tool checks useful but insufficient for goal success; F03–F05. Criterion/evidence contract, P03. |
| 1.2 | Task decomposition | agent/task_decomposition.rs; llm_classifier.rs; hive | Retain model decomposition; DAG cycle/dependency checks remain mechanical. Verify live path, partial DAG recovery and no invented subtasks, P02/P11. |
| 1.3 | Persistent task queue | agent/task_queue.rs | F02 library not wired into real goals. Transactional leases and composition fixture, P02. |
| 1.4 | Context manager | agent/context.rs | F07/F32; useful budgets do not cover final wire. P04/P14 preserve goal plus final count. |
| 1.5 | Self-correction | agent/self_correction.rs; stagnation.rs | Tool-name failure counter misses semantic stagnation and can retain growing error lists. Bound error evidence, track operation fingerprint and new evidence, preserve LLM strategy choice; P07/P11. |
| 1.6 | DONE definition | agent/done_criteria.rs; runtime.rs | F03 empty unenforced legacy object. Converge with Witness P03. |
| 1.7 | Cross-task learning | agent/learning.rs; memory | Useful structured artifacts; scope/proxy/value gaps F13. P08/P11 measure transfer and harmful reuse. |
| 2.1 | Watchdog | agent/watchdog.rs; temm1e-watchdog binary | Old manager is distinct from current process witness watchdog. Preserve restart detection; same-owner immutability overclaim F17, P03. |
| 2.2 | State recovery | agent/recovery.rs; startup.rs | Storage/startup routines do not prove in-flight recovery; queue wiring F02. Kill/restart real entrypoint P02. |
| 2.3 | Health-aware heartbeat | automation/heartbeat.rs; perpetuum/pulse.rs | Useful ticks/schedules; component health must be observed not configured. Distinguish healthy process vs progressing goal, P10/P13. |
| 2.4 | Memory failover | memory/failover.rs | In-memory fallback is bounded but not durable secondary; production construction not found. Wire deliberately, expose degraded durability and reconciliation; P08/P13. |
| 3.1 | Output compression | agent/output_compression.rs | Preserve bounded presentation; retain full evidence artifact/hash, exit code and truncation reason. Test error buried in huge output, P04/P07. |
| 3.2 | System prompt optimization | agent/prompt_optimizer.rs; context.rs | Keep reusable base, test exact prompt effects, avoid optimizing away goals. Stable-prefix cache and ablation P14/P11. |
| 3.3 | Tiered model routing | agent/model_router.rs; llm_classifier.rs | Roadmap conflicts with current single-selected-model vision. No default cheap routing; explicit Eigen exception stays opt-in, P05/P11. |
| 3.4 | History pruning | agent/history_pruning.rs; context.rs | Active path uses grouping, not full advertised semantic prune function. Replace lossy handoff F32, P14. |
| 4.1 | Discord | channels/discord.rs | Keep messaging-first interaction/file transport; test reconnect, duplicate delivery, rate-limit, role changes and message splitting P13. Live account untested. |
| 4.2 | Git tool | tools/git.rs; code_snapshot.rs | Keep real git operations. Snapshot mutates index F26; process cleanup P07, dirty repo and conflict fixtures. |
| 4.3 | Skill registry | skills; tools/skill_invoke.rs | Keep instruction packages and invocation. Define deterministic project/user precedence, reload revisions and provenance; substring relevance is not semantic guarantee. P08/P12. |
| 4.4 | Slack | channels/slack.rs | Retired upload API and incomplete paging F22; P13 replaces transport and persists ingestion cursor. |
| 4.5 | Web dashboard | gateway/dashboard.rs; router.rs | Read-only overview exists. Back with real events and authenticated scoped APIs; do not label configured OTLP as healthy. P13. |
| 5.1 | S3/R2 storage | filestore/s3.rs; local.rs | Real multipart client exists; missing abort F28. Bound/cleanup/reconcile, local compatible-service test P13. |
| 5.2 | OpenTelemetry | observable/otel.rs; metrics.rs | F20 stub transport and unbounded histograms. Wire real export/aggregation P13. |
| 5.3 | Multi-tenancy | core/tenant_impl.rs | Manager abstraction not production isolation; memory scope F13, default role F30. End-to-end principal enforcement P08/P13. |
| 5.4 | OAuth identity | gateway/identity.rs | Gateway identity scaffolding not Codex subscription. Implement only with selected cloud mode; PKCE/state/session persistence test P13. |
| 5.5 | Horizontal scaling | core/orchestrator_impl.rs | Docker factory unimplemented, Kubernetes placeholder F21. Real adapter + atomic capacity reservation before declaring shipped P13. |
| 6.1 | Parallel tools | agent/executor.rs | Helper conflicts reproduced F09, ordinary loop sequential. Effect/resource graph before enabling concurrency P07. |
| 6.2 | Agent delegation | agent/delegation.rs; cores; hive | Legacy manager vs live TemDOS/Hive. Retain main-agent authority, shared scope/budget/evidence; P02/P08/P10. |
| 6.3 | Proactive initiation | agent/proactive.rs; perpetuum | Legacy manager not construction-wired; current Perpetuum supplies newer mechanism. Durable goals/no-op maintenance fixes P02/P10. |
| 6.4 | Adaptive prompt | agent/prompt_patches.rs; anima; consciousness | Legacy patch manager not wired; distinguish current adaptation. Version patches, validate policy invariants and evaluate regression P11/P14. |
| 7.1 | Vision/images | providers; tools/browser_observation.rs; gaze | Actual image content exists; unknown-model vision=true and rough token estimates F10/F07. Capability-gated images and native platform evidence P05/P07. |

## Later feature families and cross-cutting capabilities

| Feature / idea | Vision source | Implementation | Assessment and better implementation |
|---|---|---|---|
| λ-Memory | tems_lab/LAMBDA_MEMORY.md | agent/lambda_memory.rs; memory/sqlite.rs | Preserve levels and decay; fix scope F13 and units; benchmark recall at equal context, P08/P11. |
| Engram | tems_lab/ENGRAM_MEMORY.md | agent/engram.rs; memory engram methods; tools/engram_tool.rs | Keep pinning/hysteresis/GC; checkpoint40 preserves explicit Engram/curator policy through CLI/TUI/replacement/workers (RUNTIME-POLICY-IMPLEMENTATION.md). Checkpoint43 supplies real user scope and literal-query lookup (TOOL-IDENTITY-IMPLEMENTATION.md); checkpoint44 rejects no-op backend writes and ambiguous deletion with scoped SQLite comparison (ENGRAM-PERSISTENCE-IMPLEMENTATION.md). Checkpoint42 fixes numeric inputs/packing; EMA reinforcement is not wired to writes. Cadence beyond substantive/off remains unimplemented; provenance, contradictions, finite math and scoped deletion, P08/P11. |
| Blueprints | docs/design/BLUEPRINT_SYSTEM.md | agent/blueprint.rs; tools/prowl_blueprints.rs | Keep procedural memory and piggyback hint; preconditions/version invalidation, outcome evidence, P11. |
| Finite Brain / complexity modes | VISION.md; tems_lab/TEMS_MIND_V2_PLAN.md | agent/runtime.rs; llm_classifier.rs; context.rs | Keep adaptive effort and selected model; final-token/usage invariant P04/P10/P14. |
| Consciousness | tems_lab/consciousness/IMPLEMENTATION.md | agent/consciousness_engine.rs; consciousness.rs | Keep metacognition; not independent proof of correctness. Include all cost/latency and evidence ablation P10/P11. |
| Anima/personality/ethics | tems_lab/social/IMPLEMENTATION_SPEC.md | anima facts/evaluator/user_model/ethics/personality | Keep work/word firewall, OCEAN and style optional; correct Unicode facts F29, calibrated uncertainty/privacy P08/P11. |
| Many Tems / Hive | tems_lab/swarm/DESIGN.md | hive; agent/spawn_swarm.rs | Keep stigmergy and parallel specialization. Checkpoint43 inherits caller identity/role/workspace. Batch pheromone queries; fairness/dependency/cancellation/budget evidence P02/P07/P10/P11. |
| TemDOS eight cores | tems_lab/temdos/TEMDOS_RESEARCH_PAPER.md | cores definition/registry/runtime/invoke_tool | Keep main-agent decision authority; checkpoint43 fixes active child CWD/Admin/user substitution (TOOL-IDENTITY-IMPLEMENTATION.md). Trusted standalone API, full capability scope and evidence outputs remain P08/P12. |
| Eigen collection/training/shadow/local serving | tems_lab/eigen/LOCAL_ROUTING_SAFETY.md | distill collector/curator/backends/engine/judge/stats | Keep double opt-in; fixed quantiles/SPRT and independent validation F16/F24, P11. |
| Perpetuum alarms/monitors/recurring | tems_lab/perpetuum/VISION.md | perpetuum tools/store/pulse/monitor/chronos | Keep durable time-aware activity; explicit DST/catch-up/claims, exposure fix F25, P02/P10. |
| Perpetuum conscience/volition/self-work | tems_lab/perpetuum/IMPLEMENTATION_DETAILS.md | perpetuum cognitive/conscience/volition/self_work/parking | Keep autonomous upkeep; no-op false success F06 and unaccounted calls F15. Real handlers and shared admission P10. |
| Vigil/logging/bug reports | tems_lab/vigil/DESIGN.md | perpetuum log_scanner/bug_reporter/tracing_ext; main | Keep self-diagnosis and consent; F36 Unicode panic, allowlisted/redacted reports with approval evidence and idempotent submissions, P13. No external report sent. |
| Cambium self-growth | tems_lab/cambium/CAMBIUM_RESEARCH_PAPER.md | cambium; main run_minimal_session | Keep generated extensions with fixed gates; incomplete broad pipeline F14 and scoped trust P09. |
| Witness oaths/ledger/tiers/five laws | tems_lab/witness/RESEARCH_PAPER.md | witness; agent/witness_init.rs; watchdog | Keep inability to self-mark done; real evidence, tiers/workspace wiring F04, policy pending P03. |
| Prowl browser/OTK/logins/swarm pool | tems_lab/prowl/IMPLEMENTATION.md | tools browser/session/pool/credential_scrub/prowl_blueprints | Keep credential isolation and browser reuse; login recipe is not proof of100+ live services. Typed effects, per-account contexts, expiry/failure tests P07/P08. |
| Gaze desktop | tems_lab/gaze/DESIGN.md; docs/GAZE_NATIVE_COMPUTER_USE.md | gaze; tools/desktop_tool.rs | Keep native vision/input; F31 pixel-diff interpretation, OS/monitor transform tests and panic recovery P07/P11. |
| Tem-Code edits/grep/glob/patch/snapshot | tems_lab/code/RESEARCH.md | tools/code_*.rs; file_safety.rs | Keep exact edit and read prerequisite; content-hash concurrency + temporary index/journal F26, P07. |
| MCP self-extension/manage | docs/dev/architecture.md; README self-extending tools | mcp client/manager/self_add/self_extend/mcp_manage/transports | Keep dynamic tools; pagination/typed content F27, capability provenance and process cleanup P07/P08. |
| Custom tools | README self-extending tools | tools/custom_tools.rs | Keep extensibility; validate manifest, effect/capability declaration, version rollback and isolated smoke tests P07/P09. |
| Unified web search | docs/web_search/IMPLEMENTATION_DETAILS.md | tools/web_search dispatcher/backends/cache/governor | Keep fan-out, source footer and hard caps; sort key F33, cross-source ranking calibration, bounded caching P11/P14. |
| TUI/onboarding/theme/commands | README Interactive TUI | tui crate | Preserve palette and inputs; compact transcript accepted. Actual streaming/events/steering/virtualization F34, P15. |
| API key/OTK/vault | docs/OTK_SECURE_KEY_SETUP.md | gateway/setup_tokens.rs; vault; tools/key_manage.rs | Strong random one-use TTL tokens and authenticated encryption useful; scope chat+channel/principal, private atomic writes and redaction. Same-host key access not prevented, P05/P08. |
| RBAC/allowlists | docs/RBAC.md | core/types/rbac.rs; main; channels | Keep explicit user/admin UX; fail-open defaults and blacklist growth F30. Owner bootstrap and capability enforcement P08. |
| Codex subscription/custom models | docs/setup/docker-oauth.md; README | codex-oauth; core/config/custom_models.rs | Preserve existing login/custom endpoint flexibility; account-aware catalog, refresh and quota F01/F10/F11/P05/P06. |
| Telegram/CLI/WhatsApp transports | docs/channels/telegram.md; cli.md; README | channels telegram/cli/whatsapp/whatsapp_web/common | Keep messaging/file onboarding. Explicitly separate cloud API and web-session transports; test dedup/reconnect/allowlist/multimedia and supported live routes P13. |
| Delivery tools/messages/files | README; core channel/filestore traits | tools send_message/send_file/check_messages; channels/file_transfer | Keep asynchronous communication; durable outbox, destination scope and idempotency; interrupt steering distinct from new goal P02/P13. |
| Local memory/storage backends | docs/api/traits.md | memory sqlite/markdown/failover; filestore local | Keep lightweight local setup; schema migrations, corrupt-store recovery, symlink/scope and durability fixtures P08/P13. |
| Config/install/update/deployment | docs/ops/configuration.md; deployment.md | core/config; install.sh; main update paths; Dockerfile; deploy; CI | Keep single-binary deployment and hot configuration; F35 checksum status; migration/rollback/target smoke P12/P13. |
| Provider compatibility and prompted tools | docs/dev/adding-provider.md | providers; agent/prompted_tool_calling.rs | Keep broad reach; typed protocol states, complete usage/capability resolution; prompted parser failures explicit, no success inference from prose P05/P07. |
| Compaction/prompt/application caching | creator request; core message split | agent/context/history_pruning; providers; web_search/cache | Dedicated F32/F33 audit and full algorithm P14; caching does not enlarge model window. |

## Completeness controls

The generated [source inventory](evidence/source-inventory.tsv) lists every production Rust source file by crate, and [document inventory](evidence/document-inventory.tsv) lists repository Markdown headings. These inventories make omissions visible; inventory inclusion is NOT a claim that every line received manual review. Feature-family assessments above are the broad audit. High-confidence defects have exact symbols/lines in the findings register; less deeply exercised areas have explicit acceptance work. Live services, native OS behavior, all feature flags, binary targets and historical benchmark replications remain unverified.

Preservation rule: an implementing model may repair/deprecate an unused module only after mapping its advertised behavior to a tested replacement. A library-only feature is not automatically unwanted; cloud tenancy and identity remain part of the original vision until the creator retires them.

TUI connection identity/model-switch acceptance: `TUI-CONNECTION-IMPLEMENTATION.md` (checkpoint35). Checkpoint36 adds spending continuity across TUI replacement; Checkpoint37 retains classifier usage on parse failure; Checkpoint38 binds root reconstruction budgets; remaining shared factory/connection identity is next.

Shared startup connection identity: `CONNECTION-RESOLUTION-IMPLEMENTATION.md` (checkpoint39). Root reload and runtime feature-policy propagation remain open.
