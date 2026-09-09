# Witness provider response contract

Checkpoint 54 repairs the provider adapters before connecting them to the main factory. A verifier response is a bounded structured judgment, not arbitrary prose from which a favorable JSON example may be extracted.

## Implementation

`ProviderTier1Verifier::verify` and `ProviderTier2Verifier::audit` reject empty or oversized goal/rubric/evidence inputs before calling the provider. Limits are respectively 16 KiB, 8 KiB and 64 KiB, matching the dispatcher bounds introduced in checkpoint 53. Both adapters keep the caller-selected model, send a clean context with no tools, and request at most 4,096 output tokens. This is a provider request limit; it is not a monetary reservation or a whole-wire input-token proof. Models that consume this allowance on reasoning may abstain through truncation; no successful verdict is manufactured in that case.

The remaining Tier 2 user-prompt instruction that forced PASS when no falsification was found, and FAIL for hypothetical scenarios, is replaced with the same three-valued evidence rule as its system prompt: observed contradiction means FAIL, sufficient support may mean PASS, missing evidence or an unverified scenario means INCONCLUSIVE. Tier 2 remains advisory.

Completion parsing rejects reported `length`/`max_tokens` truncation and unexpected tools, tool results or images. Opaque provider replay state is ignored as non-verdict data. Text is accumulated only up to 16 KiB. This bound applies after the provider has materialized its response; transport allocation and incomplete/ambiguous provider termination require their own provider contract.

`parse_tier1_response` accepts exactly one JSON object, optionally inside one complete Markdown JSON/plain fence. Surrounding prose, multiple objects, unknown or duplicate fields, unknown verdicts, missing/blank reasons, reasons exceeding 2 KiB, and oversized responses are rejected. Uppercase verdict spelling remains compatible. Rejection propagates through the existing dispatcher as Inconclusive. Parse failures do not echo the raw response into diagnostics. Custom verifier trait implementations remain host-controlled; their dispatcher checks are separate from this provider-adapter parser.

This intentionally removes the previous prose-extraction compatibility. A sentence saying a JSON PASS was merely an example must never become an authoritative PASS. Existing plain JSON and whole fenced JSON remain accepted. The public response shape stays `{verdict, reason}`.

## Validation

The before-test failed because the original parser accepted `Not a valid verdict: {"verdict":"pass","reason":"example only"}`. Failure is retained in `implementation-witness-contract-before.log`. The same test passes after the repair and also covers extra fields, unknown verdicts, blank explanations and oversized explanations. The old trailing-prose acceptance test now requires rejection.

Three actual provider-adapter integration tests capture both Tier 1 and Tier 2 requests. They verify selected-model preservation, output limits, no tools, evidence bytes, and valid abstention; invalid inputs cause zero provider calls; truncated, tool-bearing, oversized and prose-example responses cause errors after exactly the attempted calls. These are in-process provider-boundary fixtures, not paid model calls, HTTP transport or main CLI factory acceptance.

The final Witness suite passes 95 unit tests and 48 integration tests (`implementation-witness-contract-final-tests.log`). Full workspace/all-feature/all-target clippy passed in1m57s and the disk guard cleaned2.1GiB (`implementation-witness-contract-workspace-clippy.log`). The existing dependency future-compatibility notice remains. No user profile or paid provider was accessed.

## Next dependencies

Current main-factory Tier 1/Tier 2 flags and percentage-overhead policy are still unwired. Bind verifiers immutably to current RuntimeResources and owning metering, including model changes and delegates. Enforce bounded calls and finite validated policy before enabling new paid work. A USD-overhead allowance needs known per-goal base cost; a subscription quota is not a zero-dollar price. Preserve explicit unknownness and do not use another concurrent goal's global spending as the denominator. Whole-wire model input fitting, truthful cost readouts, reservation/cancellation semantics and durable unsuccessful attempts remain open. This checkpoint does not change those defaults or establish release readiness.
