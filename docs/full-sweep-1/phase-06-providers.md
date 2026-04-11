# Phase 6: Provider Integration

## SWEEP-601 — Anthropic hardcodes max_tokens: 4096

**Phase:** 6.1 — Response Parsing

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | Changing default max_tokens affects every Anthropic request |
| Runchanged | 30% | Long-form responses silently truncated |
| **RC** | **2.73** | |
| Agentic Core | **DIRECT** | Provider response shapes agent behavior |
| Blast Radius | GLOBAL | All Anthropic users |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Every request |
| Provider Coupling | SINGLE | Anthropic only (required field) |
| Test Coverage | PARTIAL | Test verifies 1024 from request, not 4096 default |
| User Visibility | DEGRADED | Incomplete responses, no truncation indicator |
| Fix Complexity | MODERATE | Need model-aware max_tokens from registry |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | User can ask for continuation |

**Priority Score:** (30 x 7 x 3 x 1) / (10 x 2 + 1) = 630 / 21 = **30** -> P1

**Note:** Anthropic API *requires* max_tokens. The fix is to pull the model's max output from the model registry instead of hardcoding 4096. This respects both the API requirement and the `feedback_no_max_tokens.md` rule.

---

## SWEEP-602 — Key rotation ping-pongs with no backoff when all keys exhausted

**Phase:** 6.2 — API Key Lifecycle

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Adding exhaustion detection to provider |
| Runchanged | 15% | Multiple rate-limited keys = rapid fail loop |
| **RC** | **0.94** | |
| Agentic Core | **DIRECT** | Provider key management |
| Blast Radius | GLOBAL | All users on rate-limited provider |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | |
| Provider Coupling | SINGLE | Anthropic key rotation |
| Test Coverage | NONE | |
| User Visibility | ERROR | Rapid error messages |
| Fix Complexity | MODERATE | Track last-rotation time, add minimum interval |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | MANUAL | Wait for rate limits to expire |

**Priority Score:** (15 x 7 x 5 x 5) / (15 x 2 + 1) = 2625 / 31 = **84.7** -> P1

---

## SWEEP-603 — Body sanitizer may corrupt JSON containing "data: [DONE]"

**Phase:** 6.3 — Response Parsing

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | Changing sanitizer logic |
| Runchanged | 1% | LLM response must contain literal "data: [DONE]" string |
| **RC** | **0.09** | |
| Agentic Core | **DIRECT** | Provider response parsing |
| Blast Radius | ISOLATED | Single response |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | SINGLE | OpenAI-compat only |
| Test Coverage | NONE | |
| User Visibility | ERROR | Parse failure |
| Fix Complexity | TRIVIAL | Check for SSE markers only outside JSON |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | User retries |

**Priority Score:** (1 x 1 x 5 x 1) / (10 x 1 + 1) = 5 / 11 = **0.5** -> P4

---

## SWEEP-604 — CompletionResponse always reports cost_usd: 0.0

**Phase:** 6.4

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 20% | Adding cost calc to provider layer |
| Runchanged | 0% | Budget tracker at agent layer works correctly |
| **RC** | **0** | |
| Agentic Core | INDIRECT | |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | |
| Provider Coupling | MULTI | All providers |
| Test Coverage | NONE | |
| User Visibility | SILENT | Budget tracker handles this correctly |
| Fix Complexity | MODERATE | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | N/A | |

**Priority Score:** 0 -> P4

---

## SWEEP-605 — OAuth tokens stored in plaintext JSON, bypassing Vault

**Phase:** 6.5

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Integrating with Vault encryption |
| Runchanged | 100% | All Codex OAuth users have plaintext tokens |
| **RC** | **6.25** | |
| Agentic Core | NONE | Codex OAuth crate |
| Blast Radius | ISOLATED | Single user's tokens |
| Reversibility | DEPLOY | |
| Data Safety | **CREDENTIAL** | OAuth tokens readable by anyone with filesystem access |
| Concurrency | NONE | |
| Provider Coupling | SINGLE | Codex OAuth only |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | MODERATE | Encrypt via Vault before writing |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | MANUAL | Rotate tokens after exposure |

**Priority Score:** (100 x 1 x 1 x 5) / (15 x 2 + 1) = 500 / 31 = **16.1** -> P2
