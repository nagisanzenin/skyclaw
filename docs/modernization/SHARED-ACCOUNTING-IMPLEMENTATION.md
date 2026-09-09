# Composed runtime accounting

## Defects found in actual entrypoints

Initial CLI/server composition created an unrelated BudgetTracker for TemDOS, another for the JIT Hive parent, and another inside AgentRuntime. Perpetuum's production LlmCaller returned only text and discarded provider usage. JIT workers ran with unlimited isolated budgets and transferred a scalar subtotal only after the whole swarm finished, losing subscription/unknown classification and omitting decomposition calls. A parent limit therefore did not consistently govern its delegated/auxiliary calls.

## Implemented composition

`AgentRuntime::with_budget` accepts the budget created by its owning composition. Initial CLI/server runtimes and TemDOS share that tracker. The CLI reconstruction that adds Perpetuum tools preserves it. JIT Hive context takes the active runtime's actual tracker rather than allocating another.

`BudgetTracker::child` keeps per-worker counters while forwarding each estimate, subscription or unknown outcome once to the owning budget. Worker admission consults the parent before the next call. The old final JIT scalar add is removed to avoid double-counting. Local snapshots remain useful for worker reporting; failed workers retain already recorded usage. Counters saturate rather than wrap. Multi-field snapshots are explicitly not atomic during concurrent writes; join contributors before reporting final totals. This is still a between-call estimate gate; concurrent in-flight calls have no reserved maximum charge.

`MeteredProvider` covers auxiliary consumers that do not already record usage. It checks admission before invoking the wrapped provider and records successful completion metadata once. Its owned attempt guard records an unknown outcome if an error, timeout or cancellation drops the call. Streaming retains cumulative snapshots and settles once after terminal EOF; it does not add every snapshot. Observer calls delegate to the underlying real observer implementation. Health/model-list operations are passed through without fabricated token charges. Do not wrap a consumer that already records the same call in this budget.

Both production Perpetuum initialization paths receive this wrapper, so its text-only abstraction no longer loses usage for those callers. JIT Queen decomposition also uses the wrapper, including unsuccessful decomposition after a completed response. Queen token addition now widens before adding to avoid u32 overflow.

## Acceptance and remaining boundaries

Validation passed 787 agent unit tests, 15 TemDOS unit tests and 79 root minimal-feature tests. Full workspace/all-feature/all-target clippy passed (`implementation-shared-accounting-workspace-clippy.log`), reclaiming 2.2 GiB. The root minimal-feature build retains four pre-existing cfg-dependent unused-mut warnings; they are not a failed test. Logs: `implementation-shared-accounting-final-agent-tests.log`, `implementation-shared-accounting-agent-core-tests.log`, `implementation-shared-accounting-root-tests.log`. Final worker error reporting was then refined to retain known usage on failure and was covered by the full workspace lint. Added cases check sibling admission and exact parent/local totals, unknown propagation, rejection before any provider call, cancellation after confirmed invocation, cumulative streaming settlement and subscription identity when the caller discards usage.

No disk-backed global ledger or provider-attempt reservation is claimed. The tracker remains in-process; configured/session rebuild paths beyond the composition changed here, other automatic Hive dispatch, TUI model-switch lifetime, provider-internal retries, detached/cancelled foreground calls, and all auxiliary feature paths still require complete coverage. Perpetuum instances created directly by library consumers with a raw provider remain unmetered unless they supply an accounting wrapper. Existing scalar cost reports remain known subtotals; subscription/unknown calls are not free usage. Persistent usage, profile-wide limits and principal/session policy need explicit definitions before changing restart/reset behavior.


## Checkpoint 36 — TUI owning budget and replacement continuity

A new TUI session creates one tracker before any auxiliary service starts. TemDOS receives it, AgentRuntime is bound to it, JIT uses the runtime's parent tracker and Perpetuum receives MeteredProvider. The foreground provider remains raw to avoid double accounting. The live handle owns the tracker, and both model replacement and failed-replacement restoration receive the same Arc. Changing model therefore cannot reset prior usage, unpriced calls or subscription counters and reopen admission.

The real Unix PTY fixture adds an explicitly priced synthetic model and a $0.0001 estimate limit. Its first turn consumes the allowance; after a model switch the next user turn must display budget exhaustion and make zero additional HTTP requests. The existing connection-isolation/restore/terminal fixture remains in the same run. Final results are recorded in IMPLEMENTATION-STATUS after execution. This is in-process session continuity, not durable budget persistence across process restarts. Root CLI/server alternate reconstruction and other standalone auxiliary composition are still separate work.


## Checkpoint 37 — classifier parse/cancellation accounting

The priced TUI fixture exposed a completed classifier response whose JSON parse failed. `classify_message` returned an error and discarded Usage, while its caller only charged parsed successes. The runtime now supplies a MeteredProvider with a local child budget. Provider usage settles before parsing; local counters are folded into the turn regardless of parse success, and the old success re-record is removed. The local snapshot distinguishes recorded logical outcomes from HTTP retries. Timeout/cancellation drops the owned attempt guard and records unknown once; an admission rejection creates no call record. The classifier no longer logs its full raw response text.

Regression acceptance uses the actual AgentRuntime path: valid and invalid classifier JSON each make two calls and retain exactly20 input/40 output tokens, without double charging; a blocked classifier canceled by token or runtime deadline records one unknown outcome and rejects the next limited call before invoking the provider. Agent787unit and15integration tests passed. The PTY budget limit is now $0.0002 because both real first-turn responses are charged, superseding checkpoint36's demonstration with one lost classifier charge. Final PTY/lint results are recorded after execution in IMPLEMENTATION-STATUS.

This does not make every foreground failure billable metadata available, count provider-internal retries, add durable reservations or repair other callers that pass an unmetered provider directly. `recorded_calls` counts logical recorded outcomes, not all HTTP attempts; missing usage remains unknown. Root runtime replacement continuity and other auxiliary consumers remain separate acceptance work.


## Checkpoint 38 — root command ownership and automatic Hive

CLI/server now allocate one command-lifetime budget after configuration loads. All25 root runtime constructions receive it, including OAuth startup, model/tool/provider reload, fallback and automatic Hive mini-runtimes. Worker mini-runtimes receive a local child so reporting remains per-worker; the parent gets each outcome once. Automatic Hive decomposition now uses MeteredProvider and widens token operands before addition. Failed workers retain already recorded usage/known cost instead of returning invented zeros. The aggregate Hive reply is not re-charged.

The real `scripts/cli_budget_smoke.py` fixture drives the interactive CLI with local HTTP, explicitly priced synthetic models and provider/model reconstruction via the supported proxy setup command. Unlimited mode makes4requests (2 on the replacement); an exhausted $0.0002 session makes2requests total and0 on the replacement. It checks the first actual reply, replacement configuration, budget error, selected key and clean exit. This does not claim `/model` CLI parity or live account access. Root79minimal-feature tests and an all-feature binary check passed before the final failed-worker reporting refinement; that refinement's initial u64-to-u32 type error is retained in the build log and fixed using checked saturation. Final build/CLI fixture pass; final workspace lint is recorded after execution in IMPLEMENTATION-STATUS.

These are in-process estimate limits: process restart still begins new accounting, and no new cross-principal policy or reservation ledger is introduced. Runtime reconstruction still has duplicated feature wiring beyond budgets; a shared service/factory contract remains necessary. The separate root credential/endpoint mixing found during this inspection is the next task, not claimed fixed here.


## Checkpoint 41 — optional runtime consumers obey the owner

A real CLI fixture at checkpoint40 reproduced a bypass: two classifier/foreground completions exhausted the configured USD estimate, but a substantive Engram curator still issued a third HTTP request. `scripts/auxiliary_budget_smoke.py` preserves that failing control (`implementation-auxiliary-budget-before-smoke.log`). All traffic uses temporary profiles and a local synthetic endpoint.

AgentRuntime now obtains a metered auxiliary provider for blueprint authoring, blueprint refinement, Engram curation and social evaluation. Their text/parser APIs remain unchanged. Admission happens when a queued background job actually invokes the provider. Returned usage is charged before semantic parsing; invalid JSON, an empty result or blueprint SKIP cannot erase the model call. Errors and canceled in-flight calls become unknown usage exactly once. A rejected admission invokes no provider and fabricates no call. Foreground accounting remains separate and is not double charged.

The actual CLI acceptance passes: limited2requests/0curator versus unlimited3requests/1curator, with the final foreground reply delivered in both cases. Runtime integration exercises valid and malformed curator JSON, provider error and shutdown cancellation with a known foreground tariff; final owner snapshots retain two logical outcomes, and only failed/canceled auxiliary usage is unpriced. Returned foreground TurnUsage stays a foreground result; read the owning budget after drainage for combined totals.

This does not reserve charges for concurrent in-flight calls or retroactively turn the estimate gate into a strict invoice cap. Consciousness, foreground failures, verifier/planner and other integrations still need their complete call-lifecycle audit. Logical provider calls are not retry-attempt HTTP counts. No durable global ledger or new curator cadence is claimed.

Validation: 788agent unit tests and16runtime integration tests pass, including valid/malformed/error/canceled curator accounting. Initial integration fixture compilation failed a health_check return-type mismatch; corrected, with failure retained in `implementation-auxiliary-budget-tests.log` and pass in `implementation-auxiliary-budget-tests-v2.log`. Real CLI before-control failed with1curator/3calls despite exhausted budget; after-control passes limited2calls/0curator and unlimited3calls/1curator (`implementation-auxiliary-budget-{before-smoke,after-smoke}.log`). Agent/all-feature/all-target clippy passed17.45s and cleaned1.8GiB (`implementation-auxiliary-budget-clippy.log`). No Cargo remains active. No paid account calls.
