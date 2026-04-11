# Phase 8: Concurrency & State Safety

## SWEEP-201 — ProviderConfig::Debug byte-slices API key (UTF-8 risk)

**Phase:** 8.1 — Lock Audit (found during config review)

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 5% | |
| Runchanged | 1% | API keys are ASCII in practice |
| **RC** | **0.17** | |
| Agentic Core | INDIRECT | |
| Blast Radius | CHANNEL | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | Only on debug print |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | ERROR | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | HAS_OCCURRED | Same class |
| Recovery Path | SELF_HEALING | |

**Priority Score:** (1 x 3 x 5 x 1) / (5 x 1 + 1) = 15 / 6 = **2.5** -> P3

---

## SWEEP-204 — Circuit breaker TOCTOU race in can_execute()

**Phase:** 8.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Need CAS loop or Mutex for state transitions |
| Runchanged | 5% | Multiple test requests slip through HalfOpen |
| **RC** | **0.31** | |
| Agentic Core | **DIRECT** | `circuit_breaker.rs` in temm1e-agent |
| Blast Radius | GLOBAL | Affects provider call gating |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Race window during recovery |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | PARTIAL | Single-threaded tests only |
| User Visibility | DEGRADED | Extra failed requests during recovery |
| Fix Complexity | MODERATE | CAS loop or single Mutex for state transition |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | Extra requests just fail, circuit reopens |

**Priority Score:** (5 x 7 x 3 x 1) / (15 x 2 + 1) = 105 / 31 = **3.4** -> P3

---

## SWEEP-207 — Unbounded channel for early replies (negligible)

**Phase:** 8.2

Per-message scope, 1-2 items max. No practical risk. **P4.**

---

## SWEEP-208 — Unified message channel capacity 32

**Phase:** 8.2

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 5% | Increase buffer size |
| Runchanged | 5% | Only under extreme multi-channel burst |
| **RC** | **0.83** | |
| Agentic Core | INDIRECT | Message routing |
| Blast Radius | GLOBAL | All channels block |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | Delayed responses |
| Fix Complexity | TRIVIAL | Increase capacity |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | Backpressure resolves when processing catches up |

**Priority Score:** (5 x 7 x 3 x 1) / (5 x 1 + 1) = 105 / 6 = **17.5** -> P2

---

## SWEEP-209 — Per-chat channel capacity 4 can block global dispatcher

**Phase:** 8.2

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | Need try_send or async drop pattern |
| Runchanged | 10% | Single slow chat blocks all chats |
| **RC** | **0.91** | |
| Agentic Core | **DIRECT** | Dispatcher loop in main.rs |
| Blast Radius | GLOBAL | All chats blocked |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Every dispatched message |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | All users experience delays |
| Fix Complexity | MODERATE | Use try_send with overflow queue, or increase capacity |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | Resolves when slow chat processes backlog |

**Priority Score:** (10 x 7 x 3 x 1) / (10 x 2 + 1) = 210 / 21 = **10** -> P2

---

## SWEEP-211 — EigenTune tick loop has no shutdown signal

**Phase:** 8.3

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 30% | Adding CancellationToken to the loop |
| Runchanged | 5% | Orphaned training tasks on shutdown |
| **RC** | **0.16** | |
| Agentic Core | INDIRECT | EigenTune subsystem |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | MODERATE | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | RESTART | Runtime shutdown kills tasks after 5s drain |

**Priority Score:** (5 x 1 x 1 x 2) / (30 x 2 + 1) = 10 / 61 = **0.2** -> P4

---

## SWEEP-212 — Pheromone GC loop: no cancellation, JoinHandle dropped

**Phase:** 8.3

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | |
| Runchanged | 5% | Multiple GC loops if Hive recreated |
| **RC** | **0.45** | |
| Agentic Core | NONE | |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | MODERATE | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | RESTART | |

**Priority Score:** (5 x 1 x 1 x 2) / (10 x 2 + 1) = 10 / 21 = **0.5** -> P4

---

## SWEEP-213 — Agent runtime spawns fire-and-forget tasks (no handle tracking)

**Phase:** 8.3

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 20% | Tracking all spawned handles is a moderate refactor |
| Runchanged | 2% | If a fire-and-forget task panics, features silently stop |
| **RC** | **0.10** | |
| Agentic Core | **DIRECT** | `runtime.rs` spawns |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | MODERATE | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | Panic hook logs it, features just stop working silently |

**Priority Score:** (2 x 1 x 1 x 1) / (20 x 2 + 1) = 2 / 41 = **0.05** -> P4

---

## Concurrency Summary

| Pattern | Count | Risk |
|---------|-------|------|
| `std::sync::Mutex` in async context (no deadlock, minimal hold) | 4 | LOW — guards never cross await |
| `std::sync::RwLock` in async context | 2 | LOW — same |
| Unbounded channels | 2 | LOW — scoped to per-message or TUI |
| Bounded channels (potential backpressure) | 2 | MEDIUM — can block dispatcher |
| Fire-and-forget spawns | 11+ | LOW — caught by panic hook but features degrade silently |
| Tasks without shutdown signals | 2 | LOW — drain timeout handles shutdown |
