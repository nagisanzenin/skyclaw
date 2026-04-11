# Phase 3: LLM Handling Audit

## SWEEP-016 — Token estimation underestimates CJK/Arabic by 2x

**Phase:** 3.1 — Context Window Safety

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 30% | Changing token estimator affects context budget across entire system |
| Runchanged | 10% | CJK/Arabic users hit context overflow, get 400 errors |
| **RC** | **0.32** | |
| Agentic Core | **DIRECT** | `crates/temm1e-agent/src/context.rs:50-51` |
| Blast Radius | CHANNEL | Users with non-Latin conversations |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Every message passes through token estimation |
| Provider Coupling | AGNOSTIC | Affects all providers |
| Test Coverage | PARTIAL | Unit tests only use Latin text |
| User Visibility | ERROR | API returns 400, message fails |
| Fix Complexity | MODERATE | Need Unicode-aware estimator or len/2 fallback for non-ASCII |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | User retries, history pruning eventually catches up |

**Priority Score:** (10 x 3 x 5 x 1) / (30 x 2 + 1) = 150 / 61 = **2.5** -> P3

**Note:** Elevated to P1 in summary due to Agentic Core DIRECT impact and the wide blast radius for non-Latin users. The formula score is misleadingly low because Rchange is high — but the fix is straightforward and well-bounded.

**Proposed fix:** Use `s.chars().count() / 2` as the base estimate, or detect non-ASCII ratio and adjust divisor from 4 to 2.

---

## SWEEP-017 — Memory entries injected as Role::System

**Phase:** 3.3 — Prompt Injection Surface

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 5% | Would need to change Role assignment for memory entries |
| Runchanged | 5% | Crafted memory entry could manipulate system behavior |
| **RC** | **0.83** | |
| Agentic Core | **DIRECT** | `crates/temm1e-agent/src/context.rs:279,311,353,404` |
| Blast Radius | CHANNEL | Affects single user's future sessions |
| Reversibility | DEPLOY | |
| Data Safety | READ | Could expose system prompt or other context |
| Concurrency | HIGH | Every message with memory retrieval |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | Behavior changes without user awareness |
| Fix Complexity | COMPLEX | Requires separating system context from user-derived context |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | MANUAL | Requires clearing poisoned memory entries |

**Priority Score:** (5 x 3 x 1 x 5) / (5 x 4 + 1) = 75 / 21 = **3.6** -> P3

**Note:** This is an architectural concern. The fix is complex and touches the core context builder. Marking as P2 in summary due to the security implications, but the actual exploitation requires a user deliberately crafting malicious memory entries against their own bot.

---

## SWEEP-018 — No automatic retry on 429 RateLimited

**Phase:** 3.4 — Rate Limit & Retry

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 25% | Adding retry logic to process_message — touches critical path |
| Runchanged | 15% | During API traffic spikes, every user gets errors |
| **RC** | **0.58** | |
| Agentic Core | **DIRECT** | Retry would live in agent runtime or main.rs dispatch |
| Blast Radius | GLOBAL | All users on rate-limited provider |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Every rate-limited request |
| Provider Coupling | MULTI | Affects all providers that can return 429 |
| Test Coverage | NONE | |
| User Visibility | ERROR | "Please wait and try again" — user must manually retry |
| Fix Complexity | MODERATE | Add retry-with-jitter loop, respect Retry-After header |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | MANUAL | User must retry manually |

**Priority Score:** (15 x 7 x 5 x 5) / (25 x 2 + 1) = 2625 / 51 = **51.5** -> P1

---

## SWEEP-019 — Cost tracking: provider response always $0.00

**Phase:** 3.5 — Cost Tracking

Cost calculation happens in the budget tracker (agent layer), not the provider layer. The provider's `cost_usd: 0.0` is correct because cost is a runtime concern. Budget tracking is verified working with atomic operations. Streaming responses track usage via `StreamChunk::Done { usage }`.

Gemini's fake stream (calls `complete()` internally) preserves cost tracking.

**Residual risk:** If budget tracker fails to process a response, cost is permanently wrong. No defense-in-depth.

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 20% | Adding cost calc to provider layer |
| Runchanged | 0% | Budget tracker works correctly today |
| **RC** | **0** | |
| Agentic Core | INDIRECT | |
| Blast Radius | ISOLATED | |
| User Visibility | SILENT | |
| Test Coverage | PARTIAL | |
| Fix Complexity | MODERATE | |

**Priority Score:** 0 -> P4

---

## SWEEP-020 — Tool call loop protection: verified working

**Phase:** 3.6

`max_tool_rounds` (200) enforced at loop top. `max_task_duration` provides time-based circuit breaker. Both limits are configurable and validated. No issues found.

**Status:** PASS — no finding.

---

## SWEEP-015 — Context window management: verified working

**Phase:** 3.1

`max_context_tokens` capped to `(context_window - max_output) * 0.90`. Priority-based budget allocation in `build_context()`. Well-designed system.

**Status:** PASS — no finding. (Token estimation accuracy is a separate finding: SWEEP-016.)
