# Modernization closeout A/B — September 9, 2026

## Decision evidence

The frozen original study completed all60runs: baseline30/30 artifacts passed, modernization29/30. The original strict artifact gate is **false** and remains false. Its only discordant pair is `pagination_cursor`: the prompt says “Select IDs” without specifying whether `selected` contains IDs or row dictionaries, while the external oracle requires dictionaries. Modernization returned IDs and accurately described its own successful ID-based checks. This is a contract ambiguity, not sufficient evidence of a fabricated test result or a runtime regression.

After documenting that ambiguity and freezing an explicit row-dictionary contract, one additional B-first/A-second pair ran once with identical binaries/resources. **Both passed.** The original30pairs, the29unambiguous pairs (both29/29), and the clarification pair are separate evidence. No old score was changed, no best-of retry was selected, and the clarification is not a31st independent scenario.

Engineering assessment: these results do not identify a material artifact regression under the clarified contract. Together with the [actual product acceptance](CLOSEOUT-VALIDATION.md), they support presenting a6.0 release candidate. They do **not** establish statistical equivalence, a universal quality improvement, or satisfaction of every feature vision. The raw predefined strict gate did not pass; merge/release must not be advertised as a clean30/30 modern benchmark win.

## Observations

| Measure | Baseline A | Modern B |
|---|---:|---:|
| Original artifact checks passed |30/30|29/30|
| Logical provider requests |254|230|
| Provider request failures / cancellations / pending |0 /0 /0|0 /0 /0|
| Runtime / process failures |0 /0|0 /0|
| Protected fixture corruption |0|0|
| Replies stopped at the tool-step cap |8|3|
| Sum of foreground time |2,203.619s|1,829.476s|
| Sum of observed wall time |3,558.636s|2,959.154s|
| Reported input / output tokens |478,428 /154,036|334,896 /121,768|

Wall time includes post-turn activity/drain. Initial local compilation overlapped the beginning of the experiment; shared provider load and caching were uncontrolled. Timing is descriptive, not a causal speedup claim. Baseline instrumentation does not expose cache/total-knownness fields: its254calls have unknown cache status. Modernization reports81,920cache-read tokens. That asymmetry is **not** evidence of zero baseline caching or a quantified cache improvement. Token values are observed counters, not a subscription invoice or account-wide quota measurement. There is no dollar-saving estimate.

All cases used GLM-5.3-Flash through the Z.ai Coding Plan endpoint, temperature1,4096output tokens,30,000input budget,8tool rounds,40logical requests,240s runtime,245s outer bound and380s process bound. Classifier and Witness were disabled in this matched core harness. The original case order was balanced15A-first/15B-first with seed60909. Baseline is immutable `da503c09ca5c0f41c308c99e42d4736aa3611f8a`; modern production source is `15734b7`. Later artwork/docs/version edits do not change that measured source. Matching Rust instrumentation SHA256: `c12c9ab3492295a3b51199a02ccbf440b196290c041efd7784801f7122e4d902`.

## Behavior and scope limits

Artifact correctness and a polished final reply are separate. Eleven runs stopped with an explicit tool-step-cap message despite artifacts passing the external oracle (A8/B3). This is not reported as fabricated completion. Earlier agent-written tests sometimes failed before repairs, sometimes because the test expectation or import path was wrong. Their attempts remain in the archive. Passing the final external oracle does not validate every sentence of the model’s explanation; no comprehensive “zero hallucinations” metric is claimed.

Both versions also generated out-of-workspace test/hash writes despite the harness instruction: review found explicit/tmp or default temporary-directory candidates in12A and9B runs. Examples include A pagination `/tmp/check_page.py` and B SQLite `/tmp/check_transfer.py`. Some tests require temporary directories and some attempts failed, so these counts are a trace-review inventory, not an exhaustive OS-effect audit. The shell is not an OS sandbox. Protected fixture integrity is a narrower property than compliance with every workspace instruction. No personal profile or external channel was part of the acceptance design.

This is a selected30-task standard-library coding corpus, one sample per version per task. It is not a random sample of all user workloads or an evaluation of all65feature families. Even zero losses in30independent sampled pairs would have a9.50% one-sided95% upper bound; it cannot certify a5percentage-point noninferiority margin. The29unambiguous pairs provide still less precision. Do not pool the earlier four-task pilot, relabel the clarification as independent, or infer that every experimental feature is production-complete.

## Reproduction and retained evidence

- [Frozen original design](CLOSEOUT-DESIGN.md) and [clarification rationale](CLOSEOUT-CLARIFICATION.md).
- [Machine-readable scores, summaries and archived traces](../../benchmarks/modernization/results/closeout-2026-09-09/).
- `evidence.tar.gz` preserves399 original/clarification evidence files, including prompts, calls, outcomes and workspace files; application profiles and redundant derived review files are excluded. `EVIDENCE-MANIFEST.json` hashes each archived file. The credential value was checked in memory and is absent; no credential is included.
- The summary script leaves release authorization false. Original raw score remains29/30modern, even after the successful clarification.
- [Release acceptance](CLOSEOUT-VALIDATION.md), [migration](UPGRADING.md), [feature coverage](FEATURE-COVERAGE.md) and [implementation history](IMPLEMENTATION-STATUS.md) define the broader evidence and unfinished boundaries.
