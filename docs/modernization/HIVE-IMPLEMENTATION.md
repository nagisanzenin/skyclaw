# Hive task publication and completion

## Failure and cause

Windows CI job 102180270464 (run 34261489651, browser checkpoint 29) failed `parallel_respects_dag_dependencies`: three tasks reported four completions. The task claim itself was conditional, but dependency resolution read a pending snapshot and later wrote `ready` without checking the current state. Two concurrent resolvers could therefore reset an already active task, allowing a second worker to claim and execute it. Retrying CI until green would conceal a production race.

## Implemented state rules

Readiness publication now performs a conditional `pending -> ready` update. It reports a newly ready task only when that update changed the row. A resolver resuming with a stale snapshot cannot reset an active or complete task. This preserves the creator's parallel DAG workers and dependency ordering.

Completion first performs `active -> complete` inside a SQLite transaction and updates the owning order's count/token subtotal in that same transaction. A database error rolls both back. An already complete task returns without rewriting its result, token evidence or completion count; missing/non-active tasks fail explicitly. This is idempotent completion accounting, not exactly-once execution of external tools.

Dependency publication occurs after completion commit. Ready polling also resolves dependencies, repairing a process interruption between commit and publication without rerunning the completed task. Dependency status is terminal on the normal worker path. Publication still uses the existing per-dependency checks; broad DAG admission/cycle validation and scheduling efficiency are separate work.

## Validation and boundaries

Deterministic regression cases cover a stale pending snapshot resuming after a claim, duplicate concurrent completion and late changed results, injected order-accounting failure with rollback, and injected readiness-publication interruption repaired by polling. The original parallel DAG test remains in the package suite. All 80 Hive unit tests passed; six existing benchmark tests remain ignored. Combined provider/Hive validation also passed, including 83 provider unit tests and nine HTTP/retry fixtures (`implementation-observer-hive-integration-tests.log`). Final scoped all-target lint for both packages passed (`implementation-observer-hive-final-clippy.log`), reclaiming 897.2 MiB. Initial Hive lint failed on one collapsible conditional and was corrected; that failure log is retained. Windows acceptance awaits the new revision’s CI.

This change does not provide execution-generation leases, process-crash resumption of active tasks, artifact-write transactions, global model reservations or exactly-once provider/tool effects. Existing `fail_task` and public state APIs still require a wider generation/ownership audit; these repairs must not be advertised as full durable autonomous goal management. Completion accounting is an existing token subtotal, not a complete measured-cost ledger. The original CI failure log is retained locally as `implementation-browser-windows-job.log`.
