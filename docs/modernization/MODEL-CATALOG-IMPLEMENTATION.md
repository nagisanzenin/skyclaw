# Provider-scoped model facts and billing

Checkpoint dated 2026-09-08. This replaces model-name substring pricing; it does not certify every catalog model as working through every Tem adapter.

## Failure corrected

The former matcher assigned a Sonnet tariff to unknown models, assigned zero to many `glm`/local-looking names regardless of hosting route, confused GPT-5 generations, and priced Gemini 3 variants through unrelated Flash rates. Its tests repeated those constants instead of checking billing identity and usage semantics. A provider-neutral model ID cannot identify the seller or the user's subscription entitlement.

`temm1e-core/src/types/model_catalog.json` contains exact provider/model IDs, explicit aliases, standard token tariffs, source URLs, verification date, and independently checked limits where available. Optional data stays absent. `model_catalog.rs` owns lookup and estimation; the older limits bank remains a compatibility fallback. New context facts override old limits, but saved model selection and onboarding defaults remain unchanged pending adapter acceptance.

Sources checked on 2026-09-08:

- [OpenAI standard pricing](https://developers.openai.com/api/docs/pricing), with separate short/long-context rates; [Astra model details](https://developers.openai.com/api/docs/models/gpt-6-astra) and [Sol model details](https://developers.openai.com/api/docs/models/gpt-5.6-sol) distinguish maximum input from total context.
- [Claude model overview](https://platform.claude.com/docs/en/models/overview) and [pricing](https://platform.claude.com/docs/en/about-claude/pricing). Sonnet 5 remains $2/$10 per million input/output tokens; the previously announced September increase did not take effect. Cache creation depends on retention duration, and Fable 5.1 has a different cache-read multiplier.
- [Grok 4.6](https://docs.x.ai/developers/models/grok-4.6) specifies an inclusive 200,000-input-token price threshold. Its maximum output was not established by this source and is not invented.
- [Gemini 3.8 Flash](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-flash) and [GLM 5.3 Flash](https://docs.z.ai/guides/vlm/glm-5.3-flash) supply limits. No unverified API tariff is copied from a neighboring model.

## Billing contract

`Pricing` is published, custom, subscription, or unknown. The Z.ai coding-plan and OpenAI Codex routes return subscription even when the selected model also has an API price. OpenRouter and generic proxy routes do not inherit the original model vendor's tariff. Unknown model suffixes do not inherit a family price.

`CostEstimate` is a standard-token interval, subscription, or unavailable. Token usage is normalized once by provider adapters: total input includes cache reads and writes. Let I be total input, R reported cache reads, W reported cache writes, O output. When all categories and the write duration are known:

    cost = ((I - R - W) * input_rate + R * read_rate
            + W * write_rate + O * output_rate) / 1,000,000

A price tier is selected from total input before calculating any category and applies to the whole request. When a cache measurement is missing, remaining input may occupy any compatible unmeasured category: use the smallest and largest applicable rate to construct a conservative interval. Unknown write duration retains the published minimum/maximum write rates. Missing total usage, invalid category sums, nonfinite arithmetic, or positive cache usage without a supported tariff returns unavailable. A missing cache count is not recorded as a measured zero.

These are standard token estimates, not invoices: hosted tools, storage, taxes, service-tier/geography premiums and negotiated rates are outside this estimator. A legacy scalar compatibility field carries the known upper subtotal; zero is not proof of free usage or a complete total. The structured estimate and counters retain this distinction in the budget layer.

## Runtime and migration

Foreground calls, classifier calls, compaction retries, consciousness observations and TemDOS core calls use the provider-scoped estimate. The budget records the upper endpoint, subscription-call count and unpriced-call count separately. A configured USD estimate limit rejects an unknown/subscription tariff before the first agent call; missing totals prevent later limited calls. Unlimited mode (`max_spend_usd = 0`, the existing default) still allows these providers and retains unknown billing. A USD limit does not measure subscription points or remaining quota.

The ledger rounds upward to its 1e-8 USD unit and saturates rather than wrapping. Invalid numeric limits/costs fail closed. This remains a between-call estimate gate, not a guaranteed invoice ceiling: in-flight concurrency, cancellation charges and auxiliary call paths still require a shared reservation and durable usage ledger.

Existing custom models with nonzero configured prices retain them. Legacy omitted `0/0` values no longer claim free usage; use a published tariff if known, otherwise unknown. Explicit `pricing_verified = true` can attest an intentionally zero custom tariff. `/addmodel` requires both price arguments together, rejects nonfinite values, and sets this flag only when both prices were provided. Subscription routing always takes precedence over these API-equivalent values.

## Acceptance and remaining work

Tests cover exact billing identity, aliases versus invented suffixes, subscription separation, the inclusive Grok threshold, the Astra threshold, normalized cache arithmetic, incomplete usage, interval bounds, catalog uniqueness/source metadata, counter overflow and rejection before any provider call. Test logs are recorded in IMPLEMENTATION-STATUS after execution; passing these does not establish live access to a catalog model.

Remaining: centralize the rest of the legacy capability bank and menus; implement/accept native Responses tool calling for newer OpenAI models; preserve new native reasoning items; obtain missing provider tariffs; propagate typed cost intervals through every UI/serialized metric and delegated worker; account for every background/canceled call in a durable shared budget; implement provider-authoritative subscription quota reporting. Do not advertise those as implemented by this checkpoint.
