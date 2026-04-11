# Phase 4: Channel Resilience

## SWEEP-401 — split_message() UTF-8 byte-slice panic in Telegram/Discord/Slack

**Phase:** 4.1 — Per-Channel Health

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Replacing byte-slice with char_indices — purely safer, same behavior for ASCII |
| Runchanged | 35% | Any multi-byte response near the 4096/2000/4000 split boundary |
| **RC** | **35.0** | 35 / (0 + 1) = 35. Extremely high urgency. |
| Agentic Core | INDIRECT | Channel output, not agent internals |
| Blast Radius | CHANNEL | All users on the affected channel |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Every long response |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | No multi-byte split test exists |
| User Visibility | FATAL | catch_unwind catches, but user gets no response |
| Fix Complexity | TRIVIAL | Replace with char_indices() safe split |
| Cross-Platform | UNIVERSAL | |
| Incident History | **HAS_OCCURRED** | Same bug class as Vietnamese text crash (2026-03-09) |
| Recovery Path | SELF_HEALING | catch_unwind absorbs, session continues |

**Priority Score:** (35 x 3 x 10 x 1) / (0 x 1 + 1) = 1050 / 1 = **1050** -> **P0 EMERGENCY**

**Proposed fix:** In all three channels, replace `&remaining[..max_len]` with a char_indices-based safe split that finds the last char boundary at or before `max_len`.

---

## SWEEP-402 — Non-allowlisted users get permanent silence

**Phase:** 4.2

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Adding a reply before `return Ok(())` |
| Runchanged | 80% | Every unauthorized user experiences this |
| **RC** | **80.0** | |
| Agentic Core | NONE | Channel-level handling |
| Blast Radius | ISOLATED | Single unauthorized user |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | FATAL | User thinks bot is broken |
| Fix Complexity | TRIVIAL | Send "You are not authorized" reply |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | Likely happens regularly, just unreported |
| Recovery Path | NONE | User has no way to know they're denied |

**Priority Score:** (80 x 1 x 10 x 10) / (0 x 1 + 1) = 8000 / 1 = **8000** -> **P0 EMERGENCY**

**Note:** Reclassified from P2 to P0 because of the extreme UX failure. This is the most common first-time-user experience, and it's indistinguishable from a dead bot.

---

## SWEEP-403 — Telegram backoff never resets on successful connection

**Phase:** 4.3

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Reset backoff on success |
| Runchanged | 20% | After prolonged uptime with disconnects, recovery slows |
| **RC** | **20.0** | |
| Agentic Core | NONE | |
| Blast Radius | CHANNEL | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | Slower reconnection |
| Fix Complexity | TRIVIAL | Reset backoff inside loop on success |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | |

**Priority Score:** (20 x 3 x 3 x 1) / (0 x 1 + 1) = 180 -> P1

---

## SWEEP-404 — Discord wildcard `*` allowlist not in Telegram/Slack

**Phase:** 4.4

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 15% | Inconsistent security behavior |
| **RC** | **15.0** | |
| Agentic Core | NONE | |
| Blast Radius | CHANNEL | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | ERROR | Admin expects open access, gets deny-all |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | MANUAL | Admin must reconfigure |

**Priority Score:** (15 x 3 x 5 x 5) / (0 x 1 + 1) = 1125 -> P1

---

## SWEEP-405 — Telegram no rate limit between message chunks

**Phase:** 4.5

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Add small delay |
| Runchanged | 10% | Very long responses trigger Telegram rate limit |
| **RC** | **10.0** | |
| Agentic Core | NONE | |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | Partial response |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | |

**Priority Score:** (10 x 1 x 3 x 1) / (0 x 1 + 1) = 30 -> P2

---

## SWEEP-406 — Slack no pagination for conversations.list

**Phase:** 4.6

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 5% | Only large workspaces (>200 channels) |
| **RC** | **5.0** | |
| Agentic Core | NONE | |
| Blast Radius | CHANNEL | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | Messages from overflow channels never arrive |
| Fix Complexity | MODERATE | Implement cursor-based pagination |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | NONE | No fix short of code change |

**Priority Score:** (5 x 3 x 1 x 10) / (0 x 2 + 1) = 150 -> P1

---

## SWEEP-407 — WhatsApp Web empty allowlist allows all (violates DF-16)

**Phase:** 4.7

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Change `return true` to `return false` |
| Runchanged | 10% | Security bypass |
| **RC** | **10.0** | |
| Agentic Core | NONE | |
| Blast Radius | CHANNEL | |
| Reversibility | DEPLOY | |
| Data Safety | READ | Unauthorized users can interact with bot |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | DEPLOY | |

**Priority Score:** (10 x 3 x 1 x 1) / (0 x 1 + 1) = 30 -> P2

---

## SWEEP-408 — WhatsApp Web has no reconnection loop

**Phase:** 4.8

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | |
| Runchanged | 40% | Any network hiccup kills the channel permanently |
| **RC** | **40.0** | |
| Agentic Core | NONE | |
| Blast Radius | CHANNEL | All WhatsApp Web users |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | FATAL | Bot goes offline permanently |
| Fix Complexity | MODERATE | Add reconnection loop with backoff |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | RESTART | Requires full process restart |

**Priority Score:** (40 x 3 x 10 x 2) / (0 x 2 + 1) = 2400 -> P0
