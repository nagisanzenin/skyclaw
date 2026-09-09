# Witness preserves unknown results

Checkpoint 50 corrects composite verification logic and rejects changed sealed Oaths before verification effects. It preserves the creator's distinction between a claimed answer and independently established results.

## Reproduced failures

Four before-tests exercised the real predicate/Witness APIs. AllOf converted a passing file check plus an unset time marker into Fail instead of Inconclusive. Empty composites could pass, a verdict with no required checks could pass, and changing the goal after sealing did not prevent verification. The complete failing run is retained in `implementation-witness-knownness-before-tests.log`.

## Three-valued contract

For nonempty conjunctions, a known failure is decisive; all known passes yield Pass; otherwise the result is Inconclusive. For nonempty disjunctions, a known pass is decisive; all known failures yield Fail; otherwise the result is Inconclusive. Negation preserves Inconclusive. In particular, NotOf(AllOf(unknown)) and NotOf(AnyOf(unknown)) cannot become Pass.

| Left | Right | AllOf | AnyOf |
|---|---|---|---|
| Pass | Pass | Pass | Pass |
| Pass | Fail | Fail | Pass |
| Pass | Inconclusive | Inconclusive | Pass |
| Fail | Pass | Fail | Pass |
| Fail | Fail | Fail | Fail |
| Fail | Inconclusive | Fail | Inconclusive |
| Inconclusive | Pass | Inconclusive | Pass |
| Inconclusive | Fail | Fail | Inconclusive |
| Inconclusive | Inconclusive | Inconclusive | Inconclusive |

Empty AllOf/AnyOf return Inconclusive under the non-vacuous verification policy. This is an explicit requirement-coverage policy, not a claim that classical empty conjunction/disjunction identities are mathematically incorrect. An empty check set cannot establish achievement, including through negation. The overall verdict also requires at least one non-advisory check; all-advisory sets remain unverified. A required failure remains authoritative over advisory passes.

The checks retain short circuiting on decisive known failure/pass. AllOf now continues past an unknown child to discover a decisive later failure; that can execute later authorized checks that the previous erroneous early-failure path skipped. Caller authority from checkpoint48 still gates command-containing composites before any child runs. Exceptions remain exceptions and the runtime treats unavailable verification as unverified; they are not converted into a successful logical result.

## Seal integrity

Witness recomputes the Oath hash before constructing the check context or dispatching predicates. A changed goal or predicate returns TamperDetected. This extends protection to explicit embedded Witness users, in addition to the active goal-binding check added49. Hash validation detects changes relative to the supplied seal; it is not a signature or external trust anchor against an actor who can replace both document and hash.

## Validation and remaining work

The nine-pair integration test uses actual existing/missing file checks and an unset elapsed-time marker, then checks their nested negations. Separate tests cover empty sets and negation, no required checks, changed sealed goal, and a Unix tampered command that starts no process/creates no marker. The first repaired Witness suite passes all four initial regression tests and existing unit/integration targets (`implementation-witness-knownness-after-tests.log`). Final combined suites pass 784 agent unit + 72 integration and 94 Witness unit + 38 integration tests (implementation-witness-knownness-suite.log). Actual CLI uses three passing code checks plus NotOf(AllOf(unset marker)); both unlimited turns persist Inconclusive verdicts with 4 requests/2 planners, and the capped control makes 1 planner/0 foreground/0 verdict. Snapshot hashes/original objectives and restart zero-provider inspection remain correct (implementation-witness-knownness-cli-smoke.log). Binary build passed 45.75s. Full workspace/all-feature/all-target clippy passed 1m50s and cleaned 3.4GiB (implementation-witness-knownness-workspace-clippy.log). Existing dependency future-compatibility notice remains.

Persist actual verification observations against frozen goal criteria next. Keep observations, inspected raw artifacts and complete request coverage distinct. Tier1/Tier2 currently use a workspace/subtask summary while ignoring evidence_refs; those model verdicts must not be promoted into artifact proof. Fix their evidence resolution/knownness and owning resource/accounting binding. This checkpoint does not implement those pieces, whole-process sandboxing, automatic pursuit or full release acceptance.
