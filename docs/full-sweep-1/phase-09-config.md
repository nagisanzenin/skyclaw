# Phase 9: Configuration & Startup Safety

## SWEEP-215 — Empty env var produces Some("") API key, bypasses onboarding

**Phase:** 9.1 — Config Parsing

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Treating empty string as None changes config semantics |
| Runchanged | 10% | Missing env var = empty key = all API calls 401 |
| **RC** | **0.63** | |
| Agentic Core | **DIRECT** | Config feeds into agent initialization |
| Blast Radius | GLOBAL | Provider fails for all chats |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | Startup only |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | COVERED | Test documents current behavior |
| User Visibility | ERROR | Cryptic provider errors instead of onboarding |
| Fix Complexity | MODERATE | Treat empty string as None in config expansion |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | MANUAL | User must set env var or reconfigure |

**Priority Score:** (10 x 7 x 5 x 5) / (15 x 2 + 1) = 1750 / 31 = **56.5** -> P1

---

## SWEEP-221 — Memory persist failure logged but no fallback

**Phase:** 9.3 — Graceful Degradation

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | Wiring ResilientMemory into main.rs |
| Runchanged | 5% | Only when SQLite locked or disk full |
| **RC** | **0.45** | |
| Agentic Core | INDIRECT | History feeds into context |
| Blast Radius | CHANNEL | Affected chat only |
| Reversibility | DEPLOY | |
| Data Safety | WRITE | History lost on restart |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | Until restart |
| Fix Complexity | MODERATE | Wire ResilientMemory into main.rs |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | Session continues in-memory |

**Priority Score:** (5 x 3 x 1 x 1) / (10 x 2 + 1) = 15 / 21 = **0.7** -> P4

---

## Verified PASS Items

| Check | Status | Notes |
|-------|--------|-------|
| Config default values | PASS | All sections have `#[serde(default)]` |
| Unknown keys ignored | PASS | Forward compatibility maintained |
| Startup order (tracing → panic hook → config) | PASS | Correct at `main.rs:1288-1472` |
| Missing provider → onboarding | PASS | `agent_state = None` |
| Missing vault → degraded mode | PASS | `tracing::warn!`, features disabled |
| Config validation at load time | PASS | Agent config validates; main config validates via serde types |
