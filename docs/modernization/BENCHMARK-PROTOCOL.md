# Old versus modern Temm1e: preregistered benchmark protocol

Implementation authorized by the creator on 2026-09-08. Primary model requested: `glm-5.3-flash` using the Z.ai coding-plan endpoint. A subscription is not a dollar-priced API workload; report plan usage and estimated API-equivalent cost separately, never as actual subscription charges.

## Comparisons

A is immutable commit da503c09ca5c0f41c308c99e42d4736aa3611f8a. B is the recorded modernization release commit. Same model, account route, temperature/reasoning settings, tool availability, task prompt, starting workspace/database, deadlines and maximum model/tool budget. Baseline uses its existing custom OpenAI-compatible endpoint support to access exactly the same coding-plan endpoint; this configuration accommodation is disclosed and does not backport runtime fixes.

Run two tracks: **equal-resource harness comparison** (primary) and **each version's documented default configuration** (secondary product comparison). A model upgrade comparison, if later desired, is a separate experiment. Do not attribute model quality gains to harness modernization.

## Metrics

| Metric | Definition / unit | Evaluation |
|---|---|---|
| Verified task success (primary) | tasks meeting ALL required postconditions / attempted tasks | hidden deterministic checks and blinded rubric for semantic tasks; no self-grading |
| Unsupported completion (primary) | claimed-success tasks with missing/failed required evidence / tasks claiming success; also report / all attempts | compare final claim and evidence ledger to independent evaluator |
| Recovery success | interrupted tasks completing within budget without duplicated effects / interrupted tasks | kill/restart, provider disconnect, tool timeout, quota pause fixtures |
| Context retention | required facts/constraints preserved after compaction / required facts; downstream task success separately | long tasks with corrections, pending work, evidence and three compactions |
| Tool correctness | correct tool effects / attempted effects; unexpected effects and duplicate writes counted separately | filesystem/HTTP/browser fixture observations |
| Latency | task wall time, time to first useful output, recovery time; p50/p95 | monotonic timestamps; distinguish queue/provider/tool/verification delays |
| Usage efficiency | all input/output/cache-read/cache-write/reasoning tokens and requests; usage per successful task | include classifier, consciousness, Anima, Witness, Hive/Cores, learning and background work |
| Cache behavior | cached input share, write amplification, cache cold/warm latency and actual normalized cost when known | model/provider usage metadata; unknown is not zero |
| Resource stability | peak RSS, disk growth, retained record/output bytes, orphan processes | controlled workload plus soak/restart tests |
| Memory quality | correct relevant recall, cross-user leakage, contradiction handling, harmful reuse | seeded private/public facts; fixed retrieval budgets |
| Learning/growth | held-out transfer gain; unsafe/incorrect promotions, rollback recovery | independent labels; no training/evaluation task overlap |
| TUI usability | task completion, input-to-render p95, cancellation acknowledgement, lost/duplicate events, key actions required | event replay/PTY plus creator review; aesthetics not reducible to token count |
| Connection reliability | login/refresh/model selection/quota-resume success | deterministic HTTP fixtures and separately labeled live account checks |
| Feature regressions | pass rate per covered feature family | report every family; never hide a regression in an aggregate score |

## Corpus and execution

Development fixtures can include known audit defects. Freeze a separate held-out manifest before tuning; hold-out tasks should not be used to select implementation thresholds. Start with a small live pilot to establish account/model compatibility and variance. Proposed main study: at least80 independent task scenarios with three repeats per version, stratified across coding, long-context, research/browser, memory/personalization, recovery, scheduled work and delegation. Deterministic infrastructure/credential/security faults also run offline and are not inflated into model-task success counts.

Alternate/randomize A/B order within task blocks to reduce time-of-day load bias. Restore isolated workspace, database and application caches for cold-start trials. Run warm-cache trials separately; interleaving may warm a provider cache shared across versions, so disclose this and never claim controlled cold provider cache unless the API permits it. Capture provider/model revision when exposed, region, timestamp, config hash and fixture hash. Keep immutable raw artifacts with redacted credentials.

Human judgment is used only where executable outcomes are inadequate: blinded version labels, fixed rubric, two raters on a subset, disagreements adjudicated and agreement reported. No judge gets authority to overrule a failing deterministic requirement. The same GLM model may help annotate, but its opinion is not sole ground truth.

## Statistical reporting and gates

Report paired success difference with task-clustered bootstrap confidence intervals; repeated runs of one task are not independent tasks. Report paired discordances (McNemar/exact where suitable) and per-family counts. For latency and usage report paired median changes and confidence intervals, plus p95 tails. Missing/error/timeouts are outcomes, not dropped rows. Provider-wide outages can be rerun only by a predeclared symmetric rule, preserving original results.

80 tasks is a starting design, not a guarantee of power. Use pilot baseline rates and variance to size the final study for a declared detectable effect; freeze sample/stopping rules before looking at held-out A/B results. No cherry-picked best-of-three runs. Numerical/statistical fixes get reference tests independently of A/B.

Release gates: zero known failures of deterministic identity, cancellation, false-success and budget invariants; no material regression in creator-nonnegotiable behavior; evidence of primary-task quality improvement or a justified noninferiority result alongside efficiency gains. Targets (not promised results): +5 percentage points verified success or clearly lower unsupported-completion rate; >=15% lower usage/latency at equivalent quality. Averages never excuse a new severe correctness failure. If evidence is inconclusive, report inconclusive and expand the study under the preregistered rule.

## Current status

Protocol drafted before modernization benchmarking. Baseline unit results and audit probes are evidence of defects, NOT an A/B result. Live results, sample size and scorecards will be written only after execution. Z.ai's documented supported-tool list does not currently name Tem; a successful endpoint test is technical compatibility, not official product endorsement.
