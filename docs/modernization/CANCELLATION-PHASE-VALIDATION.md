# Measure cancellation separately from durable I/O

Checkpoint 52 repairs an ambiguous acceptance test and adds Windows phase diagnostics. It changes tests/CI only; it does not claim a production cancellation or Windows process-isolation repair.

## Evidence

Checkpoint48 Windows CI34288029905 failed at cancellation.rs:120: the test's2second outer timeout elapsed. That timeout wrapped journal admission, a configured1second task deadline and final durable SQLite persistence. The original log has no mechanism/phase timings, so the precise Windows cause remains unknown. Preserve `implementation-witness-authority-ci-failed.log`; later49/50green CI is not a retrospective repair of48.

A controlled local reproduction held an external SQLite BEGIN IMMEDIATE write lock for1.2seconds before admission. The unchanged whole-call2second assertion failed even though the task deadline starts only after admission (`implementation-cancellation-phases-before-test.log`). This establishes a concrete way the acceptance test can misattribute storage delay to slow tool cancellation. It does not prove that storage contention caused the original Windows failure.

## Revised acceptance contract

Four named real-runtime scenarios replace the single three-mechanism loop: token, legacy interrupt flag, configured1second deadline, and the deadline under controlled admission contention. Each waits for the actual slow tool to start, then measures its DropProbe timestamp separately from journal completion. The tool's60second side effect must never occur.

Admission/tool start has a separate15second diagnostic bound. Token and legacy cancellation must drop the live tool future within2seconds of their signal. The deadline remains1second; after observed tool start its latest possible expiry is1second later, with the same2second cancellation allowance (3second observation bound total). Final durable completion has its own10second diagnostic bound. These are acceptance-test bounds, not new user-facing service-level promises.

The test records actual drop time inside DropProbe, so observer scheduling or later persistence cannot inflate the measured tool cancellation latency. It still verifies interrupted state, one unfinished execution, durable Outcome unknown checkpoint/tool result and absence of the delayed effect. There is no detached stop task left waiting forever if admission ends before the fixture starts; failures identify that phase directly.

Production admission is currently outside the task deadline/cancellation select, and durable finish is awaited afterward. This checkpoint preserves that existing behavior. End-to-end admission cancellation/deadline semantics and safe transaction-outcome reconciliation remain separate production work; the phase distinction must not hide that boundary.

## Results

All4scenarios pass (`implementation-cancellation-phases-after-tests.log`). In this local run, token cancellation was5.375µs, legacy17.048ms, ordinary deadline991.179ms and contention-case deadline998.069ms. The contention case spent1.265s in admission and3.666ms in final persistence. These are fixture observations, not distributional latency benchmarks or native Windows measurements.

Agent all-target clippy passed17.77s and the disk guard cleaned824.2MiB (`implementation-cancellation-phases-clippy.log`). Formatting/diff checks pass and Ruby's YAML parser accepts the modified workflow. No product source or CLI behavior changed, so no new CLI/binary acceptance is claimed. Full workspace validation remains that of51;52's Windows run is pending.

Windows CI now runs the focused cancellation target with --nocapture before the full existing workspace test step, exposing phase timings on successful runs as well as failures. The broad Windows gate is retained. Inspect the actual52run before concluding that the Windows acceptance issue is resolved. These tests measure an owned async tool future; they do not validate Windows JobObjects or arbitrary child-process cleanup.


Subsequent CI evidence: checkpoint52 run34291280676 is fully green. Windows focused timings are token20.7µs, legacy40.053ms, deadline1.003s; contention admission1.275s, deadline997.642ms, persistence8.329ms, all4cases passed. See implementation-cancellation-phases-windows-ci.log. This validates the revised Windows acceptance contract without inventing the exact cause of48.
