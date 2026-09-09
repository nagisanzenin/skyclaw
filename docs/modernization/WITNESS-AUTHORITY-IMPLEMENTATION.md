# Witness command authority

Checkpoint48 closes a caller-role bypass in automatic Witness verification. When automatic planner Oaths were enabled, a User unable to call the shell tool could still cause a model-proposed CommandExits check to start a process. A real Unix AgentRuntime fixture reproduced it by creating an owned temporary marker through the verifier.

## Contract and implementation

AgentRuntime now binds a cloned Witness to the actual workspace and authenticated session role for each turn. The process-wide Witness and another in-flight turn are not mutated. The binding can narrow existing authority but cannot elevate an already restricted binding.

CommandExits, CommandOutputContains, CommandOutputAbsent and CommandDurationUnder require the caller's shell capability. Detection recursively covers AllOf, AnyOf and NotOf before executing the composite. A forbidden command makes that predicate Inconclusive; it is not run and is not inverted into a pass by NotOf. Executed tier0_calls do not count denied checks. Independent safe file/directory predicates continue to run.

Explicit legacy Witness::new and low-level predicate APIs remain trusted host APIs. Embedded integrations processing restricted users must call for_authority(workspace, role), as the production AgentRuntime now does. for_workspace alone preserves the binding's existing authority; it is not an authorization API.

The old module comment claiming verification never mutates files or processes was false: command/network checks can have external effects. It now distinguishes narrative-only reply composition from verification effects. Workspace binding is not an OS sandbox.

## Evidence

Before repair, actual User-role AgentRuntime plus automatic planner/Witness wrote the temporary marker despite shell denial (implementation-witness-authority-before-test.log). The unchanged test now passes: no marker, an Inconclusive verdict and zero executed command checks.

Unix positive/negative tests exercise all four arbitrary-program variants and the three logical wrappers. User creates no marker; rebinding that User witness as Admin cannot restore authority; an independently bound Admin creates the expected marker. DirectoryExists still passes for User. A cross-platform missing-program fixture additionally proves direct and composite commands are rejected before platform process lookup, with explicit permission detail and zero executed checks. Windows-native successful command effects are not claimed by the Unix marker fixture.

All seven binding/accounting/authority integration tests pass. Full local suites pass780agent unit+71agent integration and94Witness unit+33Witness integration tests (implementation-witness-authority-suite.log). Actual CLI Admin regression passes4unlimited requests/2execution-bound Oaths and1capped planner request/0foreground requests (implementation-witness-authority-cli-smoke.log). Binary build passed46.11s. Full workspace/all-feature/all-target clippy passed1m49s and cleaned3.4GiB (implementation-witness-authority-workspace-clippy.log); existing dependency future-compatibility notice remains. All processes/files are owned local fixtures; no live account calls or real user data were used.

## Limits and next work

This repairs one actual privilege path, not full process isolation or a complete capability registry. Controlled Git/process/network/file predicates retain their existing policies; broader sandbox/SSRF/path/effect review remains separate. Authorized Admin commands can still change workspace files or have external effects, and verification should not be described as universally read-only.

Persist execution-bound Oath proposals and actual assessment observations into the goal ledger next. Preserve evaluator type/version, original objective, scoped immutable evidence, required/advisory distinctions and unverified coverage. Do not promote arbitrary model prose or weak criteria into verified overall achievement. Queue/recovery/child lineage and D01/D02 defaults remain separate.
