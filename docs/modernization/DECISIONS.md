# Creator decisions

## Confirmed

- Source of vision: use the repository's vision and feature documents; do not ask the creator to reconstruct them from memory.
- Research the six named harnesses, finding Grok Build and ZCode online.
- Audit all feature families through ideas, implementation and math; propose concrete replacements/repairs where justified.
- Include subscription/coding-plan login, compaction/caching, and TUI modernization.
- TUI: **compact transcript first, expandable tools and optional panels**, preserving Tem palette/personality. Confirmed by creator during this operation.
- Start on a new branch. This documentation branch is `codex/modernize-temm1e-research`; no runtime redesign has been shipped.

## D01 — Personal-host autonomy versus shared-host isolation (pending)

Earlier architecture promises deny-default workspace isolation. v5.1.1 intentionally grants broad filesystem access. Proposed resolution: explicit personal-host and isolated-server profiles; personal owner keeps broad authorized capability, shared hosts require principal/workspace boundaries. Alternative: full host access everywhere. This changes product/security semantics; no default is changed by this audit. The question was sent to the creator and remains unanswered.

Implementation can proceed on carrying explicit identity/workspace/effects without selecting a new default. Shared-host production claims remain gated on isolation tests.

## D02 — Witness delivery versus durable completion (pending)

Witness Law5 protects delivery; the perpetual-pursuit vision requires continued work until actual achievement. Proposed resolution: deliver useful partial results, keep goal incomplete, and recover/continue when possible. Alternative: a narrative verdict with no automatic continuation. No silent status-policy change is authorized by the research document. The question was sent and remains unanswered.

Implement typed evidence and separate delivery/outcome fields first. Automatic continuation/default stopping behavior waits for the creator's decision.

## D03 — Subscription backends (proposal; integration eligibility required)

Preserve native Tem as primary runtime. Add supported account routes and optional official harness bridges only where capabilities and product terms support them. Do not replace Tem's mind with an external harness by default. Third-party login restrictions are factual integration constraints, not an unsolicited approval flow. Current subscription support must be verified before advertising a connector.

## D04 — Autonomous growth and routing (preserve current vision)

Preserve selected-model default and Eigen's explicit double opt-in. Preserve Cambium's intended autonomy within protected zones; do not quietly expand zones or infer trust from unrelated successful tasks. A stricter evidence gate may reduce autonomous deployment frequency; record that tradeoff in implementation PRs, and ask creator before changing the default autonomy policy.

No pending decision prevents implementing local correctness fixes, accurate telemetry, explicit skipped states, durable records or source-backed documentation.

## D05 — Modernization value and release comparison (confirmed September 8, 2026)

The creator explicitly clarified that fundamental architecture, implementation tightening and new capabilities are the purpose of modernization. Beating main on optimization benchmarks is not required. A/B should guard against meaningful regressions; verified structural and feature improvements plus production readiness can justify the authorized main merge and v6.0 release. This supersedes the earlier inferred requirement for primary-task or efficiency gains. Keep uncertainty visible and test all existing feature families; four development scenarios alone do not establish broad nonregression. See the dated amendment in `BENCHMARK-PROTOCOL.md`.
