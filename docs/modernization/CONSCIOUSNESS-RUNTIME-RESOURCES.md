# Consciousness current-runtime resources

Checkpoint61, September9,2026.

## Problem and contract

ConsciousnessEngine stored its construction-time provider/model. Attaching that engine to another current runtime left observer calls on the old route. Runtime recorded only successful observer usage after returning; failures/cancellation escaped the owner meter. The actual before-regression processed two code-shaped turns and observed four calls to the obsolete provider.

ObservationState now owns the existing notes, counter and post insight behind shared Arc/Mutex state. `for_runtime` creates a private immutable view using current provider/model/pricing with the same state. At turn admission runtime captures one view using its auxiliary MeteredProvider and uses it for both pre/post observation. Concurrent creation of another view cannot change an existing view's route. This does not clone AgentRuntime or its owned task lifecycle.

MeteredProvider performs owner admission and records success/error/drop before observer response parsing. Both former runtime record_estimate sites are removed; returned successful observer token/cost fields still fold into TurnUsage. Counter increment saturates rather than wrapping/panicking. No observer default or selected model changes. An explicitly attached engine follows its owning runtime model inside AgentRuntime; direct standalone engine calls retain their constructor resources.

## Validation and reproduction

- `tests/consciousness_resources.rs`: actual Runtime, old observer provider versus current queued provider, two turns. Before4obsolete calls; after0obsolete/6current/6owner, all request models current, post insight and earlier note retained in next pre prompt.
- Actual runtime failure/disabled controls: enabled failure path has2failed observer attempts plus foreground,3owner records; disabled has0observer/1foreground. Pending pre future dropped after provider admission records1unknown attempt and dispatches no foreground call.
- Full agent suite794unit+80integration passes;3existing ignored docs. Logs implementation-consciousness-resources-before.log, tests.log, agent-tests.log. No live provider calls.
- Actual CLI factory acceptance passes: minimal CLI+TUI build47.42s(four existing conditional warnings); script consciousness_resources_smoke.py captures real HTTP from isolated CLI/profile. Enabled pre/foreground/post twice retains trajectory; disabled2foreground; finite priced owner permits1pre call which exhausts the threshold and blocks foreground. Log implementation-consciousness-resources-cli.log.
- Full workspace/all-feature/all-target lint passed1m50s and cleaned3.8GiB; existing proc-macro-error2 future-compat notice retained. Native Windows checkpoint60CI pending separately; no Windows success claim.

## Limits and next implementation work

This repairs resource ownership and accounting, not all Consciousness semantics. Observer state retains its prior engine-wide scope; concurrent sessions can still mix trajectory, so introduce explicit workspace/user/conversation scoping with migration/default review before claiming principal isolation. Notes/output/input and raw model text remain unbounded at this engine; separate timeout/output/context validation is needed. Mode/confidence/intervention settings are not enforced by this LLM engine. Return-value TurnUsage still represents successful responses, while the owner budget records unsuccessful unknown attempts. Provider internal retry attempts and per-goal USD reservations are not represented by logical counters. Cost display still has legacy scalar placeholders. Direct standalone engine callers must supply their own meter. Reconstructing a runtime with a fresh engine can reset state or omit configuration; Perpetuum and Eigen-Tune composition remain separate.
