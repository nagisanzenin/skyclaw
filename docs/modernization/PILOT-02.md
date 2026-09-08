# Development pilot 02 — usage observation repaired, release superiority unproven

Run September 8, 2026. Main A: `da503c09ca5c0f41c308c99e42d4736aa3611f8a`. Modern B: `f26588b6fd31d2567e88ce6ea9eb8791c5877499`. Both frozen binaries used exactly the same `modernization_pilot.rs` instrumentation, GLM-5.3-Flash coding-plan endpoint, tool set, 30,000-token context budget, 4,096 output-token request cap, eight tool rounds and balanced interleaved order. This is the same four development scenarios, one attempt per version, with fresh in-memory state. It excludes the classifier, Witness and channels.

Unlike pilot 01, both processes observed post-turn provider activity for up to 125 seconds, requiring a one-second quiet interval. All 27 A and 29 B recorded requests completed; no started request lacked a terminal event and none was pending at report time. This measures the instrumented core runtime, not every production subsystem or account billing.

| Scenario | Main verified | Modern verified | Main foreground seconds | Modern foreground seconds |
|---|---:|---:|---:|---:|
| Invoice totals | yes | yes | 54.142 | 83.513 |
| Dependency ordering | yes | yes | 43.219 | 53.074 |
| Recursive merge | yes | yes | 83.094 | 56.844 |
| Interval union | yes | yes | 76.548 | 47.835 |
| Total | 4/4 | 4/4 | 257.003 | 241.266 |

Modern foreground time is 6.1% lower in aggregate, with two faster and two slower scenarios. Total observed runtime including post-turn work is 510.058 seconds for main and 483.477 seconds for modern (5.2% lower). There is no demonstrated quality gain. Four development tasks cannot establish statistical superiority or noninferiority, and neither timing difference meets the proposed 15% efficiency target.

All recorded requests together used 48,249 input / 25,149 output tokens for main and 52,075 input / 21,073 output tokens for modern. Input increased 7.9%; output decreased 16.2%; their unweighted sum decreased 0.3%. These are different resources and must not be collapsed into a subscription-cost win. Baseline's normalized Usage type does not retain cache counts; its missing cache count is unknown, not zero. Modern recorded 9,728 cache-read tokens. Shared server cache and service load are uncontrolled, so cache efficiency is not comparable from these artifacts. Reported `cost_usd=0` is not a claim of free usage or an account bill.

The verified artifacts and protected files passed all external checks. This is artifact correctness, not a blanket claim that every sentence of the transcripts was truthful: for example, modern's invoice run initially described two commands as successful despite a missing `md5sum` executable, then corrected the verification approach. A dedicated assertion-to-evidence evaluator remains necessary.

**Decision: no merge or release.** Preserve both pilots, complete implementation and production integration checks, then freeze the broader independent held-out study. Do not pool these repeated development tasks as independent release evidence.
