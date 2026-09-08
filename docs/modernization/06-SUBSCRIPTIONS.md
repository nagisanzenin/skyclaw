# Subscription login, coding plans and model access

Treat an account connection as a product feature, not an API key with a different label. Coding subscriptions, metered API access and third-party hosted plans can expose similar models with different endpoints, entitlements, usage windows and permitted integrations. Never advertise one as interchangeable with another.

## Verified landscape, 2026-09-08

| Product / route | What primary sources establish | Tem recommendation |
|---|---|---|
| Codex / ChatGPT login | Codex supports ChatGPT account login and API-key access, with headless/device options described separately. | Make a first-class account connection; keep subscription transport capabilities distinct from public API. Prefer a documented supported integration surface and validate current contract. [Auth](https://learn.chatgpt.com/docs/auth), [app server](https://learn.chatgpt.com/docs/app-server). |
| Claude Code / Claude subscription | Claude Code login exists. Agent SDK documentation restricts third-party products offering claude.ai login/rate limits without approval. | Do not promise arbitrary third-party Claude subscription login because an OSS client implements it. API/cloud-provider routes are separate; product approval or a clearly supported official-harness integration is a prerequisite. [Authentication](https://code.claude.com/docs/en/authentication), [SDK overview](https://code.claude.com/docs/en/agent-sdk/overview). |
| Grok Build | Official coding harness; Grok FAQ documents shared product usage pool. | Research official integration/entitlement before native subscription adapter; record quota scope shared with other usage. Do not infer that an xAI API key spends the same pool. [Build](https://docs.x.ai/build/overview), [FAQ](https://docs.x.ai/grok/faq). |
| ZCode / GLM Coding Plan | ZCode distinguishes account/coding-plan setup from general API configuration. Z.ai documents coding endpoints and supported tool scope. | Separate `zai-coding-plan` connection and endpoint from general Z.ai. Tem is not named in the published supported-tool list reviewed; verify eligibility rather than equating wire compatibility with support. [ZCode configuration](https://zcode.z.ai/en/docs/configuration), [Z.ai tool integration](https://docs.z.ai/devpack/tool/others). |
| Pi | Public source has subscription adapters and process-locked credential refresh. | Reuse engineering patterns, not an inference of first-party permission. [Source](https://github.com/earendil-works/pi/tree/b2602be77cb7b0de45dd616407fd210daa48aa75/packages/coding-agent). |
| OpenCode | Provider-specific connection choices include coding plans and its own hosted routes. | Show exact connection identity and entitlement in model picker. [Providers](https://opencode.ai/docs/providers/). |

No real login, subscription entitlement, quota exhaustion or billing test was performed in this audit. Provider rules and model catalogs are time-sensitive and must be rechecked before implementation/release. Native Tem runtime remains the recommended default; an official harness bridge is a separately selected backend with explicit capability differences, not a silent replacement of Tem's cognitive systems.

## Baseline Tem defects

Codex OAuth is a real implementation with PKCE/state validation, callback handling and token storage; do not dismiss it as absent. Its Responses serializer loses volatile instructions (F01). The refresh mutex coordinates one provider object, not independent processes; token files are overwritten directly before best-effort chmod (F11). Built-in models and pricing are scattered and stale (F10). Z.ai general API default can be overridden, but there is no first-class plan/account/quota contract (F12).

The current headless flow is callback-paste based, not proof of a standards-based device grant. Gateway user identity OAuth and model-provider subscription OAuth are different systems. A fresh token is evidence of authentication, not proof that the selected model or endpoint is entitled.

## Data contract

Use opaque IDs in configs; secret material lives only in the credential store.

```rust
// Proposed types, not compilable drop-in code.
struct Connection {
    id: ConnectionId,
    provider: ProviderId,
    route: RouteKind, // ApiMetered | CodingSubscription | Local | HarnessBridge
    endpoint: EndpointId,
    account_ref: Option<AccountId>,
    credential_ref: CredentialRef,
    support: SupportStatus, // Verified(date, source) | Experimental | Unsupported(reason)
}
struct ResolvedModel {
    connection_id: ConnectionId,
    model_id: String,
    revision: Option<String>,
    capabilities: Capabilities,
    entitlement: Entitlement, // Available | Unavailable(reason) | Unknown
    catalog_source: String,
    checked_at: Timestamp,
}
struct QuotaWindow {
    scope: QuotaScope, // account/product/model; may be shared outside Tem
    unit: QuotaUnit, // tokens, requests, credits, percent or provider-defined
    used: Option<Decimal>,
    limit: Option<Decimal>,
    resets_at: Option<Timestamp>,
    observed_at: Timestamp,
    source: QuotaSource,
}
```

Capabilities include context/output limits, input/output modalities, native/prompted tools, tool-result types, reasoning/phase items, streaming, cancellation, cache policy, compaction, supported parameters and usage fields. Missing metadata is Unknown. A model name alone is not a globally unique capability key.

## Account state machine and errors

`Disconnected -> Authorizing -> Authenticated -> Ready` after entitlement resolution. Authorizing may become Cancelled/Expired. Ready may become Refreshing, QuotaLimited, Revoked or TemporarilyUnavailable. Refreshing returns Ready only after successful refresh; invalid_grant becomes ReauthRequired. A 429 may mean throughput or quota exhaustion; use provider payload/headers to distinguish. A 401 is not a cue to rotate through random accounts indefinitely.

Structured errors: `LoginCancelled`, `LoginExpired`, `StateMismatch`, `RefreshConflict`, `ReauthRequired`, `ModelNotEntitled`, `QuotaExhausted { windows }`, `RateLimited { not_before }`, `UnsupportedCapability`, `ServiceUnavailable`, `UnknownUsage`. Preserve request IDs and redacted error bodies for diagnosis.

A paused goal retains its checkpoint. If reset time is known, scheduler can resume within the existing user-authorized goal and its policy. If unknown, show unknown and offer account/model choices. Switching to metered API requires an explicit user choice because billing changes. Do not create multiple-account rotation to evade provider limits.

## Login and credential implementation steps

1. Select connection kind before requesting credentials. Explain plan vs API billing concisely in onboarding. Never ask for refresh tokens as ordinary chat input.
2. For documented OAuth clients, generate PKCE verifier/state per attempt with cryptographic randomness; bind callback to that attempt, validate redirect/state, enforce timeout and one-use completion. Only implement documented device grants; headless fallback must not pretend a callback paste is device flow.
3. Keep browser callback listener loopback-bound where applicable. Device code and URI show expiry/cancel. Don't log URL fragments, authorization codes or tokens.
4. Use OS credential storage where supported; otherwise create private files atomically with a per-account process lock. Lock, reread latest generation, refresh once, write+fsync+rename, unlock. Detect another process's newer generation instead of overwriting it. Handle refresh-token rotation and process crash.
5. Implement redacted Debug/Display and explicit logout/revoke behavior. Best-effort remote revoke must report failure separately from local credential deletion. Clear account-specific cache and catalog references on logout.
6. Fetch account/model metadata through supported routes, annotate source and age, and expose last-known data when offline. Never fabricate plan name/reset time.
7. Keep server quotas separate from Tem's local budget ledger. A shared subscription can be consumed elsewhere; refresh observations and display their timestamp. Unknown remaining quota is not unlimited.

## Models and upgrades

Seed a versioned catalog from dated provider sources and merge supported account discovery plus explicit user overrides. Model IDs remain stable user choices; catalog refresh does not silently migrate them. Mark retired/unsupported IDs with actionable alternatives. The modernization operation should not hardcode another quickly outdated bank as its main fix.

When a protocol introduces reasoning/phase or encrypted compaction items, preserve them as typed provider state through retries and continuation. Cross-provider migration rebuilds from canonical user/tool evidence and portable handoff; provider-private state is not portable. Tool-call capability and image support are probed/declared per connection, not guessed from substrings.

## Acceptance scenarios

| Fixture | Expected result |
|---|---|
| Two processes refresh same account simultaneously | one valid persisted token generation; no lost rotation |
| Process dies during write | old or new complete file, never truncated JSON |
| Callback wrong state / expired attempt | reject; no credential saved |
| Login succeeds, model unavailable | authenticated account plus clear unavailable model; no false ready status |
| Quota exhausted with known reset | goal checkpointed, quota pause visible, no paid fallback |
| Quota missing | Unknown status; no fabricated percentage |
| Stream errors after partial usage | partial result retained; usage/reconciliation recorded |
| Switch API to subscription same model name | endpoint/capabilities/pricing resolved anew |
| Logout during request | request lifecycle and revocation explicitly reconciled |
| Cached prompt call | normalized reads/writes/ordinary tokens balance; no invented subscription dollar savings |
| Catalog refresh removes model | active turn snapshot stable; subsequent selection explains retirement |

Release gate: official support status recorded for each exposed subscription integration, deterministic fixtures pass, and a separately logged real-account smoke establishes current login/refresh/model/usage behavior. Without live access the deliverable is an implementation-ready design, not a certification of subscription compatibility.
