# Context compaction and caching audit

Added explicitly at the creator's request. Scope: active request compilation, historical pruning, long-term memory, provider prompt caches, application caches, usage/billing and repeated-compaction behavior. These mechanisms solve different problems; cached tokens still occupy model context.

## What exists and what is missing

| Mechanism | Baseline evidence | Assessment / proposed change |
|---|---|---|
| Recent/older history budgeting | `agent/src/context.rs` | Useful fractional budgets and atomic tool grouping. F07: final wire request is not bounded after later additions. Make one final compiler authoritative. |
| Dropped-history summary | `generate_dropped_summary` | First three user prefixes plus tool names cannot preserve constraints, corrections, blockers or evidence. Replace with versioned structured handoff backed by raw-event IDs. |
| Chat digest | `build_chat_digest` | Helps user-text continuity, but excludes tool evidence. Keep as optional view, never the sole task state. |
| Importance pruning | `history_pruning.rs` | Keyword/recency scoring helper has tests; whole prune function not production-wired. Do not add another independent pruning policy. |
| Output compression | `output_compression.rs` | Bounded presentation is useful; persist original output with hash before truncation, retain error/exit status and artifact links. |
| λ / Engram / Blueprints | memory + agent modules | Retrieval complements compacted context; cannot guarantee retrieval of every active instruction. Pin active task facts separately, preserve scope. |
| Stable prompt prefix | `message.rs`, `anthropic.rs:107` | Base/volatile split already implemented. Preserve this investment; Codex OAuth loses volatile content (F01). Deterministically order tool definitions and avoid timestamps in reusable base. |
| Anthropic caching | base block `cache_control: ephemeral` | Actual support exists. No read/write accounting fields; streaming Usage contract also insufficient. Add complete usage events and cache policy resolved by model/endpoint. |
| Gemini caching | `gemini.rs:183` flattens system | No explicit cache-object lifecycle found in adapter. Implicit cache may still hit. Support explicit only for endpoint/model that documents it and when reuse pays for storage. |
| OpenAI-compatible / Codex | adapter request building | Compatibility does not establish every cache option or compaction endpoint. Probe capabilities, keep API and subscription contracts distinct. |
| Web search cache | TTL + capacity in `web_search/cache.rs` | Useful bounded cache; F33 sort omission. Add byte bound, full semantic key, expiry telemetry; never cache mutable tool success as a reusable action. |
| Blueprint classification hint | existing classifier | Saves an extra matching call but dynamic classification still consumes tokens. Report its actual overhead, not an unqualified zero-cost claim. |

## Current provider reference points

OpenAI documents exact-prefix caching and model-specific cache controls; newer models expose both cache reads and writes, so retain both and resolve options from capability metadata. Measure hits, writes, latency and cost. [OpenAI caching](https://developers.openai.com/api/docs/guides/prompt-caching).

Anthropic reports ordinary input, cache creation and cache reads separately. Total processed input is their sum; pricing must apply category-specific rates and TTL. The baseline ignores two of these categories. [Anthropic caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching).

Gemini's current Interactions API supports implicit caching; explicit cache objects belong to generateContent. Stable prefixes and endpoint-specific usage parsing matter. Tem's generateContent adapter must use that endpoint's schema, not copy Interactions field names. [Gemini caching](https://ai.google.dev/gemini-api/docs/caching).

OpenAI's compaction API can return an opaque encrypted item together with retained items; its returned window should be preserved as the canonical continuation. Do not turn the opaque item into text, discard surrounding returned items, or assume it transfers across providers. Verify subscription endpoint support separately. [OpenAI compaction](https://developers.openai.com/api/docs/guides/compaction).

## P14 implementation: one compaction service

Dependencies P02, P04, P05 and P10. New core types and an agent service; keep provider-specific opaque items in provider codecs.

`CompactionRecord { id, schema_version, principal_id, goal_id, source_event_start, source_event_end, source_hash, model_revision, created_at, handoff, opaque_provider_state, input_usage, output_usage }`.

`handoff` contains: original goal; latest scope/corrections; accepted preferences/authorizations with source IDs; decisions; completed work with evidence IDs; current plan; unresolved questions; failures and attempted remedies; workspace/branch and artifact hashes; pending tool operation IDs. Observed tool/web data stays labeled as evidence, not promoted into user authority. Secret values are never embedded. Capture active user instructions verbatim where practical; retain original event references regardless.

Algorithm:

1. Append every event before considering compaction. Freeze a source high-water mark; newly arriving user steering stays outside the frozen segment.
2. Estimate complete next-request tokens. Start compaction before capacity minus output/reserve/maximum expected next-tool-result; these are configurable budgets, not a universal 80% magic threshold.
3. Choose completed turn groups through the high-water mark. Never compact a pending call apart from its result. Keep newest steering, active goal constraints and unresolved operations in a mechanically assembled pinned block.
4. If supported, use provider-native compaction and preserve its canonical window. Otherwise ask the selected model for the structured handoff under a bounded reserved budget. Do not silently select a cheaper model.
5. Validate schema, source references, coverage of pinned facts and artifact existence/hashes. A model summary is fallible: missing a pinned fact fails validation. Preserve raw history. On failure retain old context and retry once with specific missing fields; then pause recoverably if no fit is possible.
6. Atomically commit record and context pointer only if source hash/generation still match. Append later events after the compacted prefix. Re-run final token gate on the exact outbound representation.
7. After successive compactions rebuild durable facts from canonical events/ledger, not solely from the previous summary. Expose compacted source range and an on-demand expand/recall action.

Acceptance fixtures: correction changes deployment target; authorization withdrawn mid-compaction; user constraint appears after first 50 bytes; test failed after a successful build; completed tool pairs; unresolved browser submission; CJK/emoji; source prompt injection; three consecutive compactions; provider/model change; crash between summary creation and pointer commit. All pinned facts must survive exactly; no fabricated success; final context fits. Measure downstream task success separately from schema validity.

## P14 implementation: cache correctness and economics

Add `CachePolicy { mode, ttl, scope_key, prefix_revision }` resolved by provider/endpoint/model, and optional normalized usage fields `input_total, input_uncached, input_cache_read, input_cache_write_by_ttl, output, reasoning, storage_cost`. Missing means unknown, not zero. Persist raw usage metadata for audit without credentials. Normalize Anthropic's additive categories and OpenAI's total/subset categories differently, with fixtures preventing double counting.

Build stable instructions/tool schema once per capability revision. Hash deterministic serialized prefix; log only hash and token counts. User profile/mode/time and fresh retrieval follow the stable prefix. Never retain an obsolete policy just to preserve a cache hit. Cache identity includes account/tenant, model revision, protocol options, schema and relevant content version. Expired explicit cache IDs recreate safely; logout deletes owned remote cache where supported and clears local references.

For a reusable prefix of T tokens with write rate w, read rate r, ordinary rate p, n uses and storage charge S, estimated benefit is `n*T*p - (T*w + (n-1)*T*r + S)` in consistent units. This simplified formula excludes misses and provider-specific tiers; measure real bills and use actual TTL/reuse distribution. Do not pad useless text to cross a cache minimum. Caching is an optimization, never a reason to bypass context or account limits.

Acceptance: two stable-prefix requests have byte-identical reusable content; a policy/schema change invalidates it; no cross-principal cache IDs; synthetic cache-write/read responses reconcile exactly; unknown usage remains unknown; sorted search requests never collide; byte capacity holds under oversized search results; cache disabled and expired paths remain correct. Live cache hit/latency/billing benchmarks remain outstanding until a real provider account is available.

## Implementation update — September 8, 2026

The table above records the baseline audit. Normalized cache-read/write usage and bounded stream framing are now implemented for compatible and native Messages providers; deterministic visible tool ordering is in the runtime. The structured-handoff implementation, validation, live results and precise remaining limitations are documented in [Context implementation](CONTEXT-IMPLEMENTATION.md). Explicit provider cache-object management and complete session restart continuity are still unfinished. Neither passing schema tests nor a reported cache count establishes a latency/cost win.
