# Immutable verifier model limits

Checkpoint 57 makes each configured Witness provider retain the selected model's context/output limits for its turn and respect small custom-model output limits.

Previously, the adapter requested4,096output tokens regardless of a smaller declared model limit, while the wrapper re-read the custom-model context window on every call. A configuration edit during verification could therefore change admission inside a single turn. This contradicted the per-turn resource binding introduced in55.

`WitnessProvider::new` now resolves and stores context window and maximum output once. For each request, the allowed output is the minimum of the requested limit, the captured model output maximum and half the captured context window. The half-window ceiling preserves input space, matching the main runtime's conservative policy for models advertising output equal to their entire window. Zero usable output rejects the request before consuming an attempt or entering the provider. The existing final input estimate uses the captured window and the reduced output reserve.

This is immutable model-limit admission, not a full endpoint-aware `ResolvedModel` implementation. Pricing, account entitlements, provider wire conversion, tokenizer-exact input counts and image costs remain separate. No selected model or user profile is changed. Smaller limits can legitimately cause a reasoning model to exhaust output and abstain; truncation never becomes a PASS.

## Validation

An isolated child-process fixture uses a temporary `TEMM1E_DATA_DIR`, avoiding process-global environment changes during the parallel suite. It exercises the real custom-model loader and provider wrapper. Before the repair it failed because the selected model's1,024-token limit became a4,096-token request (`implementation-witness-limits-before.log`).

After the repair, the fixture confirms1,024-token output, edits the model file to a256-token context/128-token output, and verifies that the existing provider retains its original limits while a newly constructed provider uses128. A zero-window/output model is rejected with no extra provider call. The complete agent suite passes794unit and77integration tests; three existing documentation tests remain ignored (`implementation-witness-limits-agent-tests.log`).

Actual minimal CLI+TUI build passed47.32s, with four existing feature-conditional unused-mut warnings. The existing CLI fixture now has `--small-output-limits` so model-switch acceptance can assert exact1,024/512token HTTP limits rather than only a generic4,096cap. The actual CLI fixture passes across model change: captured HTTP requests use1,024then512output tokens, six total requests (two each planner/foreground/verifier), saved scoped evidence and restart inspection with zero calls. The finite-budget control still stops after one planner. See `implementation-witness-limits-cli-small.log`. Full workspace/all-feature/all-target clippy passed1m47s and cleaned3.7GiB (`implementation-witness-limits-workspace-clippy.log`); the existing dependency future-compatibility notice remains. No paid account or real user profile was accessed.
