# Preserve the product; repair the guarantees

The creator directed this audit to the repository's vision and feature documents. Treat those as evidence of intended outcomes, while recognizing that some implementation claims and numerical arguments were model-authored. A proposal below is not permission to delete a feature or silently choose between contradictory principles.

## Product contract recovered from the documents

| Principle | Source | Modernization constraint |
|---|---|---|
| Sovereign autonomous executor; persist until the objective is fulfilled | [VISION](../../VISION.md), pillars I and V | Distinguish an ended inference turn from an achieved goal. Persist remaining work and reasons for waiting. |
| Recover indefinitely from failure | VISION II and VI | Save authoritative state before side effects; explicitly reconcile uncertain outcomes after restart. Do not promise exactly-once effects for arbitrary shell commands. |
| Rust infrastructure; LLM judgment | VISION III and VI; [Perpetuum vision](../../tems_lab/perpetuum/VISION.md) | Code enforces ordering, budgets, identity, permissions, clocks and evidence integrity. The chosen model interprets intent, relevance, strategy and semantic success. |
| Maximum quality per resource cost | VISION IV | Evaluate cost per verified task, latency and false completion, not raw token savings in isolation. |
| Single user-selected model | VISION VI | No default cheap/expensive model router. Vary context and supported reasoning settings; preserve explicit Eigen-Tune opt-ins. |
| Finite working memory with recoverable long-term memory | VISION VI–VII; [λ design](../../tems_lab/LAMBDA_MEMORY.md); [Engram](../../tems_lab/ENGRAM_MEMORY.md) | Preserve full artifacts durably and retrieve them under a real final request budget. Compaction and λ storage solve different problems. |
| Learn through reusable artifacts, with drains | VISION VII; [artifact value](../../tems_lab/ARTIFACT_VALUE_FUNCTION.md) | Version, scope, evaluate and retire artifacts. Retrieval frequency is not proof of correctness. |
| Messaging-first, cloud/local, sovereign ownership | [original architecture vision](../TEMM1E_VISION.md); ADRs 003/004 | Subscription login, recovery, approvals and delivery must work from Telegram/Discord and a headless host, not just a desktop UI. |
| Honest personality with constructive disagreement | [Anima architecture](../../tems_lab/social/TEM_EMOTIONAL_INTELLIGENCE_ARCHITECTURE.md) | Preserve Tem's voice and relationship continuity. Personality must not alter evidence or declared success. |
| Verification without destroying useful work | [Witness research](../../tems_lab/witness/RESEARCH_PAPER.md), Law 5 | Deliver partial artifacts with accurate status. A failing verification must not delete files or suppress all useful output. Continuation policy is a creator decision. |
| Self-growth within protected boundaries | [Cambium zones](../lab/cambium/PROTECTED_ZONES.md) | Keep trust zones, independent verification and recoverable rollout. This research is not Cambium self-modifying its immutable kernel. |

## Preserve these distinctive features

- **λ-Memory and Engram:** fading episodic memories plus durable user/project facts. Add scope, provenance, invalidation and recall tests, not a replacement with generic rolling summaries.
- **Blueprints:** reusable procedures with prerequisites, verification, failure modes and execution feedback. Revalidate external dependencies and use observed outcomes for promotion.
- **Perpetuum:** time-aware concerns, monitors, initiative and sleep work. Make every activity durable, budgeted and honestly reported; retain LLM interpretation of significance.
- **Consciousness:** reflective observation and adaptation. Treat reflection as optional evidence-informed context, not another authority that can rewrite task outcomes.
- **Anima:** identity, user modeling and anti-sycophancy. Scope user data and test constructive disagreement in multiple languages.
- **Hive and TemDOS:** decentralized coordination and stable specialists are separate product ideas. Share execution/evidence services without collapsing their distinct coordination policies.
- **Prowl and Gaze:** browser/desktop capability using the selected model, OTK credential separation, layered observation and visual verification. Retain modality choices; test platform limits explicitly.
- **Vigil:** self-diagnosis and consented bug reporting. Persist report state and redact before persistence and external publication.
- **Cambium:** capability growth inside a protected substrate. Replace aspirational stages with actual gates and reversible deployment behavior.
- **Eigen-Tune:** optional local distillation from the user's own workload. Preserve collection/training research; require task-valid evaluation before calling a graduate equivalent to its teacher.
- **Tem-Code, MCP, skills and channels:** precise edits, read-before-edit, discoverable capabilities and convenient communication remain useful foundations.

## Contradictions to resolve explicitly

1. Early architecture says deny-by-default workspace isolation; v5.1.1 executor comments deliberately grant full filesystem access. This is a policy decision, not automatically a regression to reverse.
2. VISION VI rejects model routing, while the older roadmap celebrates tiered routing and Eigen-Tune can graduate local models. Proposed reconciliation: one selected model by default; only explicit, visible opt-in substitution.
3. VISION says verify every action and never stop prematurely. Witness Law 5 protects delivery and the runtime returns after a narrative verdict. Proposed reconciliation: artifact delivery and goal status are separate; an incomplete goal can continue without blocking delivery.
4. The Enabling Framework rejects cognitive heuristics, while λ/Engram/Blueprints use explicit mathematical scores. Proposed reconciliation: deterministic resource accounting and transparent fallbacks remain; formulas prioritize candidate material, and the LLM decides its semantic use. Do not delete these mathematical systems under a blanket “no heuristics” rule.
5. “Process death loses nothing,” “never overflows,” “zero downside,” and “zero cost” are absolute claims. Preserve the aspiration by defining measurable fault boundaries. Local compute, disk, quotas and verification latency are costs even when incremental API spend is zero.
6. Tem-Code research declares compaction redundant because Skull exists. The observed context assembly does not establish a hard bound or preserve all durable task state. Add compaction as a bounded working-memory operation while retaining λ/Engram as long-term storage.
7. Engram preserves facts across sessions; the original product also supports many users/channels. Explicitly shared memories must be distinguished from private user/tenant memories. Cross-session continuity must not imply cross-user visibility.

See [DECISIONS](DECISIONS.md). No unresolved choice here is silently applied to runtime configuration.
