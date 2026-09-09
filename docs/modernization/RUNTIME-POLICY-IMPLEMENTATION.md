# Runtime policy continuity

Checkpoint 40 fixes user-selected feature settings being lost during construction.

## Reproduced defect

At checkpoint 39 only two server constructors supplied Engram configuration and blueprint notices. CLI, TUI, most replacements and Hive workers reverted to AgentRuntime defaults. A real CLI fixture configured Engram disabled and curator off, sent two ordinary messages longer than 40 characters with a provider/model replacement between them, and observed six provider requests instead of the expected four. The pre-change failure is retained in `implementation-runtime-policy-before-smoke.log`; no user profile or paid account was used.

## Contract and implementation

`agent/runtime_policy.rs::RuntimePolicy` snapshots v2 optimizations, self-audit, parallel blueprint phases, blueprint notices and the complete Engram configuration. It contains no credentials or mutable resource handles. `AgentRuntime::with_policy` applies the snapshot without replacing budget, limits, tools or services; `runtime_policy` captures the effective parent settings, including subsequent explicit builder overrides. Existing individual builder APIs remain compatible.

The command captures policy once after configuration load. All 25 root constructors apply it, including credential/model/tool replacements and automatic Hive workers. TUI initial construction and replacement derive it from their retained setup configuration. SpawnSwarmContext carries the effective parent policy; each JIT worker applies its own clone. No worker enables recursive Hive routing through this policy.

Explicit Engram disablement now suppresses automatic permanent-block injection and automatic curation. Curator off suppresses automatic curator calls even when permanent-memory injection is enabled. Existing stored facts remain intact; explicit memory tools and other memory subsystems are separate controls. Default settings are unchanged.

## Acceptance

`scripts/runtime_policy_smoke.py` uses real CLI and POSIX TUI processes against local HTTP fixtures. Each temporary profile contains a pinned synthetic global fact so it is visible regardless of the fixture's local principal. Negative and positive controls run two turns separated by actual model replacement: the sentinel is absent from both models' requests when disabled and present for both when enabled. All four cases pass with exactly four classifier/foreground requests and no curator requests. The TUI additionally verifies Unicode input, resize, clean exit and exact terminal-attribute restoration. This is not a Windows-native terminal acceptance test.

`runtime_policy::tests` checks descendant inheritance of effective overrides, a frozen configuration snapshot, nondefault Engram limits and unchanged budget identity. Broader test/lint results are recorded in the status and handoff once complete.

## Remaining implementation work

This is a shared feature policy, not a complete AppServices factory. Consciousness, Perpetuum, EigenTune, provider-bound tools and other resource hooks still have separate construction paths. Do not clone BackgroundTasks naively: its Drop cancels owned tasks.

The curator currently only schedules the literal `substantive` mode and uses a greater-than-40-byte user-text heuristic. Configuration comments also advertise `every:N` and `session-end`, which are not implemented. This checkpoint preserves eligibility semantics; typed cadence validation, short durable-fact capture and explicit session-end scheduling need a separate implementation and acceptance fixture. The existing best-effort background curator is not crash-durable. Disabling Engram is not an all-memory privacy switch.

## Final checkpoint validation

Validation: 788agent unit tests,15runtime integration tests,79root tests and50TUI tests pass. All four real CLI/TUI memory-policy positive/negative controls pass through model replacement, exactly4requests each; TUI terminal attributes restore exactly. Logs: `implementation-runtime-policy-{agent-tests,entry-tests,after-smoke}.log`. The pre-change CLI failure (6requests with curatoroff) is retained in `implementation-runtime-policy-before-smoke.log`. Full workspace/all-feature/all-target clippy passed1m47s and cleaned2.9GiB (`implementation-runtime-policy-workspace-clippy.log`); existing proc-macro-error2 future-compatibility notice remains. No Cargo remains active; no paid account calls.
