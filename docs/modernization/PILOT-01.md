# Development pilot 01 — inconclusive, not a release gate

Run September 8, 2026. A: `da503c09ca5c0f41c308c99e42d4736aa3611f8a`. B: `db6ee6bbeff434e0e935d111ab0ea05a3f3f13f0`. Exact shared instrumentation: `crates/temm1e-agent/examples/modernization_pilot.rs`; corpus/order/config: `benchmarks/modernization/pilot.py`. Four standard-library coding scenarios, one attempt per version. Same GLM-5.3-Flash coding-plan endpoint/account, limits, tools and fixture; no classifier, Witness or channels in this core-runtime pilot. Both used fresh in-memory databases. This is not full-product parity or held-out release evidence.

| Scenario | Main verified | Modern verified | Main seconds | Modern seconds |
|---|---:|---:|---:|---:|
| Exact invoice totals | yes | yes | 80.57 | 58.56 |
| Dependency ordering | yes | yes | 101.16 | 50.93 |
| Recursive document merge | yes | yes | 53.20 | 89.73 |
| Interval consolidation | yes | yes | 89.07 | 122.64 |
| Total | 4/4 | 4/4 | 324.00 | 321.86 |

All external deterministic checks passed, including protected-file checks. Aggregate foreground wall time differs by only about 0.7%; directions vary by task. Four scenarios and one run each do not establish superiority, equivalence, or a population failure rate. Shared provider cache and time-varying service load remain uncontrolled. No primary quality gain was demonstrated, so this does **not** authorize merge or release.

The meter captured 26 completed requests per version: main 62,971 input / 12,196 output tokens; modern 63,530 input / 10,409 output tokens. **These token totals are incomplete.** Seven main and five modern requests had started but lacked a terminal event when the process ended after its foreground turn. Background authoring/curation can outlive the returned reply. Their account usage is unknown, not zero. Therefore no token-efficiency or subscription-cost improvement is claimed. This finding motivates tracked background task ownership, cancellation/drain, and complete usage attribution before the next study.

Next: correct background lifecycle/accounting; rerun a development pilot with an explicit bounded drain and preserve every interrupted request as an unknown-usage outcome. Freeze and execute the independent broader corpus only after implementation and comparison instrumentation are ready. Keep the original pilot artifacts and do not overwrite unfavorable/missing outcomes.
