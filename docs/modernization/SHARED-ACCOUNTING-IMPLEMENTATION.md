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
