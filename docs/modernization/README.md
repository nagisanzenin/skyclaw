# Operation Modernize Temm1e

Research baseline: **2026-09-08**, TEMM1E **v5.8.1**, commit `da503c09ca5c0f41c308c99e42d4736aa3611f8a`. Working branch: `codex/modernize-temm1e-research`.

## Recommendation

Preserve Tem's identity and its distinctive systems. Modernize the contracts connecting them: durable execution, evidence, context, model capabilities, credentials, resource accounting, and delivery. A wholesale rewrite would discard useful work without proving better outcomes. Adding another layer of prompts would leave the principal defects intact.

The highest-value initial changes are: preserve volatile instructions in subscription requests; stop reporting no-op maintenance as completed; wire durable tasks into actual entry points; bind Witness to real evidence and the correct workspace; and replace scattered model/auth assumptions with resolved, account-aware capabilities. These are correctness and integration changes before they are model upgrades.

## Reading order

1. [Vision and feature traceability](01-VISION.md): what to preserve; which documents disagree.
2. [Frontier harness research](02-HARNESSES.md): Codex, Claude Code, Grok Build, ZCode, Pi, OpenCode.
3. [Findings register](03-FINDINGS.md): concrete evidence, impact, confidence, and remediation IDs.
4. [Target architecture](04-ARCHITECTURE.md): boundaries, state transitions, compatibility, and migration.
5. [Implementation packets](05-IMPLEMENTATION.md): ordered, bounded tasks with acceptance checks.
6. [Subscription and model implementation](06-SUBSCRIPTIONS.md): account login, coding plans, billing, refresh, catalogs, protocol support.
7. [Evaluation and validation](VALIDATION.md): observed baseline, reproductions, and release gates.
8. [Creator decisions](DECISIONS.md): unresolved choices; proposed behavior is not an approved product change.
9. [Sources](SOURCES.md): dated primary sources and pinned repository references.
10. [Feature coverage](FEATURE-COVERAGE.md): review depth and remaining validation for feature families.
11. [Mathematical audit](MATH-AUDIT.md): formulas, assumptions, calibration and replacements.
12. [Compaction and caching](07-CONTEXT-CACHING.md): dedicated audit and P14 implementation.
13. [TUI modernization](08-TUI.md): creator-selected layout and P15 implementation.

## Scope and evidence discipline

This is a research and implementation-specification branch. It does not implement the proposed runtime redesign, change production defaults, connect real subscription accounts, or deploy Tem. Temporary local probes exercise the existing code; their source and results are retained under `evidence/`.

Evidence labels:

- **Reproduced**: a local probe exercised actual code and recorded the result.
- **Source-confirmed**: inspected implementation and call sites establish the stated behavior; no live service incident is inferred.
- **Design gap**: architectural mismatch or missing contract; impact requires a representative experiment.
- **Documentation defect**: a claim is contradicted by arithmetic, source, or a newer document.
- **Unverified**: neither source inspection nor an executed test establishes the claim.

The user reported that early models produced unreliable work. This audit does not infer intentional deception by a model or author. It identifies false success reports, missing integration, inadequate evidence, and unsupported guarantees. Existing benchmark reports are historical observations, not independent proof of current production quality. Competitor documentation establishes supported behavior, not superiority on Tem's workloads.

The default branch includes changes through July 4, 2026; it is not simply an untouched six-month-old snapshot. Findings apply to the pinned commit, not every past or future release. Local library tests are not an exhaustive audit of all 25 crates, native platforms, deployment modes, or provider accounts.


Coverage: 35 original roadmap entries plus 30 later/cross-cutting feature families; 36 numbered findings; 15 implementation packets. Coverage is design/source review with explicit live/platform limits, not universal production certification.
