# Modernization implementation status

Work is active on `codex/modernize-temm1e-research`. Baseline: `da503c0`; research: `a62e0a9`. This is an implementation checkpoint, not a release or a claim that modernization is complete.

## Implemented at this checkpoint

- Codex Responses requests preserve volatile system instructions.
- Explicit `zai-coding-plan` connection uses the coding endpoint, rejects accidental general-API routing and account rotation, and appears in TUI onboarding. General Z.ai connections remain available separately.
- Opt-in live provider and agent examples use a credential file supplied by environment variable. A GLM-5.3-Flash agent run created a file through the shell and passed an independent file-content check (12.92 s, 1,236 input / 258 output tokens). This is a smoke test, not comparative evidence.
- Shell output is bounded while pipes are drained. Unix timeout and cancellation kill the owned process group; regression tests check late descendant effects. Windows cleanup remains best effort pending stronger ownership.
- Search cache keys include sort order. Unicode report truncation avoids a byte-boundary panic.
- Missing maintenance handlers return typed skipped outcomes instead of reporting completed work. Actual session cleanup and blueprint refinement still need implementation.
- Removed the empty post-inference DONE reminder. Durable evidence-based completion remains unfinished.
- Provider responses preserve reported cache reads/writes; Anthropic input totals include both additive cache categories. Streaming, aggregate usage and cache pricing remain unfinished.
- Eigen-Tune sample caps produce inconclusive outcomes without graduation; terminal SPRT decisions stop sampling. Power estimation uses the correct one-sided power quantile and alternative variance. These are approximate one-sample calculations, not the paired A/B release analysis.
- New banner and feature artwork follow the creator's pixel-art brief. Illustrations are conceptual and contain no unverified benchmark claims. README integration is in progress.

## Second implementation checkpoint

- `TEMM1E_DATA_DIR` now selects application-owned state across CLI/TUI, credentials, OAuth, user configuration, browser sessions, channel allowlists, vault, Witness, Cambium, Perpetuum, skills and cores. The default remains `~/.temm1e`. Explicit paths and workspace/system configuration retain their existing meaning; this is not a sandbox. See [isolated profiles](ISOLATED-PROFILES.md).
- Credentials and OAuth tokens use atomic private-file replacement. Unix temporary files are owner-only before content is written; persistence errors propagate. Debug output omits secrets. Cross-process OAuth refresh locking and Windows ACL validation remain unfinished.
- The TUI materializes only visible transcript rows on redraw. Tool start/completion events carry distinct execution IDs through the real agent bridge, including repeated calls to the same tool. Tool rows are compact by default; Ctrl+T expands/collapses details, Ctrl+O controls the optional activity panel. Provider text streaming and broader interaction polish remain unfinished.
- Installer checksum verification now fails closed for absent/malformed/mismatched checksums or missing hash utilities, including fallback binaries. Three focused tests cover successful exact-filename verification and failure paths.
- README is reduced to 100 lines, with a linked feature guide, CLI reference and preserved historical release notes. Eighteen matching concept images follow the creator's art direction and omit numerical performance claims.

## Validation

Focused library checks: provider 83, OAuth 21, distillation 150, maintenance 79, process ownership 3 and tools 339 passed. Counts overlap with workspace tests and must not be summed as an independent total. Workspace library regression run: **2,730 passed, 0 failed, 9 ignored across 24 suites**. The subsequent full workspace run passed **2,971 tests, 0 failures, 13 ignored across 78 suites**, including binary/integration/doc targets. All-feature/all-target clippy passed with `-D warnings`; Rust reports a separate future-compatibility warning in third-party `proc-macro-error2`. Interactive entrypoint parity, migration validation, platform checks and the A/B release gates are still pending.

## Still required

P02–P15 are not complete. Follow `05-IMPLEMENTATION.md`, the issue register and feature coverage matrix for the remaining durable goals, evidence, final request budgeting, model/credential lifecycle, process/tool contracts, scope isolation, Cambium gates, background budgets, math, composition, integrations, context/caching and transcript TUI work.

A/B is preregistered in `BENCHMARK-PROTOCOL.md`; no old-versus-modern result has been established. The creator authorizes an eventual 6.0.0 release and merge only if objective comparison improves and the result is ready for existing users. Until then, main and release tags remain unchanged. Push verified checkpoints to this branch so the creator can monitor progress.

The creator is away and requests autonomous work. Preserve existing personal-use behavior when resolving routine choices; document material changes and compatibility. Compact transcript with expandable tools and optional panels is confirmed. No parallel-agent authorization has been received.
