# Configured Witness model verification

Checkpoint 55 connects an explicit bounded-call verification policy to the active AgentRuntime attachment factory. The selected provider/model and owning budget are captured anew for each turn; existing shared Witness attachments are not mutated.

## Configuration and migration

The existing defaults do not start new model-verification calls. The old main factory ignored `tier1_enabled`, `tier2_enabled` and `max_overhead_pct`. It now preserves tier flags and exposes an explicit alternative policy:

```toml
[witness]
enabled = true
tier1_enabled = true
tier2_enabled = false
model_verification_max_calls = 2
```

`model_verification_max_calls` is optional. Missing means the USD-percentage path remains in a truthful Tier-0 fallback because per-goal USD reservation is not implemented. Startup diagnostics state this limitation. Explicit zero disables configured model verification. Values 1–8 allow at most that many combined Tier1+Tier2 provider attempts per turn. This is a call allowance, **not** a subscription account quota, invoice cap, or assertion that `max_overhead_pct` was satisfied. It intentionally selects an alternative to the percentage policy. The planner has its own existing admission and metering; it is not included in this verifier-only allowance. Missing evidence and disabled tiers still make no verifier provider calls.

This opt-in is useful for coding subscriptions without inventing token prices. An owning finite USD budget can still reject subscription or unknown-priced calls; use the application's existing unlimited-USD setting only when choosing provider quota controls. No real user configuration is modified by the implementation or fixtures. Existing TOML/YAML omitting the new field remains readable; Rust `WitnessConfig` struct literals must include the field or use `..Default::default()`.

Both file loaders and direct attachment construction reject nonfinite/negative percentage values and call limits greater than8, including when Witness is disabled. Unknown monetary cost is not coerced to zero.

## Runtime binding

The factory stores tier policy in Witness. At turn start, `for_authority` creates the existing workspace/role-bound copy; the runtime binds missing configured tiers to its current provider/model. Explicit custom verifier attachments are retained. The per-turn wrapper shares one atomic remaining-call count between configured tiers, performs the existing final context estimate with10% margin and output reserve, requires the selected model/no tools/output1–4096, and bounds each provider future to30seconds. The estimate is not a model-tokenizer or provider-wire proof.

The inner provider is the existing owning `MeteredProvider`: successful usage is recorded before the verifier parses JSON; failures and cancellation retain unavailable usage once. Rejected input/call admission does not enter that meter; an attempted wrapper call rejected by the owner's budget makes no underlying HTTP call. Call counters are logical admission/verification diagnostics, not transport-retry counts. An aborted attempt does not restore its allowance. The unchanged global budget remains a threshold check, not an atomic monetary reservation.

A replacement runtime using the same attachment object captures its own provider/model and allowance. No shared mutable verifier handle is rebound. This also follows the existing root/TUI attachment composition, but separate real TUI switching/delegate acceptance remains necessary before claiming every path verified.

## Cost presentation and persistence

Legacy `Verdict.cost_usd` remains in serialized records to avoid rewriting ledger hashes. Its zero placeholder is not presented as free usage. The new accessor returns unavailable when a model verifier ran, and zero only for zero model-verifier calls. The readout explicitly labels this as model verification cost; it excludes planner calls, local compute and other turn work. Active runtime tracing uses this knownness-aware accessor. Full token/pricing/subscription measurements remain in the owning budget, not this legacy scalar. Historical ledger schemas and recorded bytes are unchanged.

## Validation status

Initial implementation integration checks pass. The first expanded suite failed to compile three new fixtures because QueuedMockProvider uses Tokio mutexes; replacing `.lock().unwrap()` with `.lock().await` repairs only those fixtures. The second suite passed792agent unit tests but the new positive factory fixture failed: its planner draft omitted the required anti-stub predicate, so no Oath was sealed and no verifier ran. The fixture now includes the same full deterministic preconditions as existing acceptance. Both failures remain in `implementation-witness-policy-tests.log` and `implementation-witness-policy-tests-v2.log`; neither is reported as a pass.

## Remaining work

Implement a per-goal USD reservation policy with valid base-cost knownness before activating percentage admission. Subscription quota discovery, exact whole-wire input fitting, durable per-attempt measurements, attempted-verifier vs actual transport counters, and unsuccessful verification persistence remain separate. Grounded model judgments still do not establish full request coverage or proof of arbitrary test execution. The broader A/B, supported-platform/channel migration and release gates remain open.

Final combined validation passes792agent unit+77integration,281core unit (one existing ignored) and95Witness unit+48integration. The additional failure/drop accounting test passes with the two existing wrapper tests (793agent unit cases now defined). These verify atomic admission, zero-call rejection, current-model factory binding across5configurations×2runtime models, owning accounting before parse, and cancellation/failure unknownness without refund or double charge. See `implementation-witness-policy-tests-v3.log` and `implementation-witness-policy-accounting-tests.log`.

Actual CLI build with TUI passed1m31s. The enabled fixture changes model through the supported `proxy` command and makes6requests:2planners,2foreground,2verifiers, each using the expected current model. Both assessments preserve the inspected file bytes and restart inspection makes0requests. Zero/omitted call-policy controls make4requests/2planners and retain Inconclusive model checks; all finite-owner-budget controls make1planner and no foreground/verifier calls. Logs: `implementation-witness-policy-cli-enabled-v2.log`, `-zero.log`, `-default.log`. The first enabled CLI fixture used `/model`, which this CLI chat path forwards as user input rather than its supported proxy configuration command; this produced an extra unsealed turn and fixture HTTP errors/retries. That failed run is retained in `implementation-witness-policy-cli-enabled.log`; the corrected test uses the established actual CLI configuration flow. Do not claim this tests the server/TUI slash-command flow.

Checkpoint53 CI34295232801 is fully green. Checkpoint54 CI34295726293 failed in a separate Markdown-memory visibility assertion: store returned while an async append was still pending, so the synchronous before-read saw only a newline and a later read saw the existing generic note. Failure retained in `implementation-witness-contract-ci-failed.log`. This requires a production Markdown flush review next; it is not reclassified as a Witness parser failure or a pass. Full workspace/all-feature/all-target clippy for55 passed1m44s and cleaned6.0GiB; the existing dependency future-compatibility notice remains.
