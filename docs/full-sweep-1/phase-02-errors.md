# Phase 2: Error Handling Integrity

## SWEEP-010 — Swallowed errors in EigenTune trainer

**Phase:** 2.1 — Swallowed Errors

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 20% | Adding error handling to async store ops |
| Runchanged | 3% | Store write failures silently lost |
| **RC** | **0.14** | |
| Agentic Core | INDIRECT | EigenTune feeds distilled knowledge into agent |
| Blast Radius | ISOLATED | EigenTune subsystem only |
| Reversibility | DEPLOY | |
| Data Safety | WRITE | Training state can become inconsistent |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | TRIVIAL | Add `tracing::warn!` on Err |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | MANUAL | Stale state requires manual cleanup |

**Priority Score:** (3 x 1 x 1 x 5) / (20 x 1 + 1) = 15 / 21 = **0.7** -> P4

---

## SWEEP-011 — Swallowed error in Perpetuum cortex store update

**Phase:** 2.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | |
| Runchanged | 5% | Concern error counter not persisted |
| **RC** | **0.45** | |
| Agentic Core | NONE | Perpetuum leaf crate |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | WRITE | Error counter lost on restart |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | RESTART | Error counter resets, concern fires again |

**Priority Score:** (5 x 1 x 1 x 2) / (10 x 1 + 1) = 10 / 11 = **0.9** -> P4

---

## SWEEP-012 — Swallowed error in classification outcome recording

**Phase:** 2.1 — **[INTENTIONAL]**

Classification outcome is non-critical analytics. Swallowing is documented as intentional. No action needed.

---

## SWEEP-013 — Swallowed errors in custom tool subprocess I/O

**Phase:** 2.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Changes process I/O handling |
| Runchanged | 5% | stdin/stdout/stderr failures produce empty output |
| **RC** | **0.31** | |
| Agentic Core | INDIRECT | Tool output feeds into agent context |
| Blast Radius | CHANNEL | LLM sees empty tool result |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | LLM misinterprets empty tool output |
| Fix Complexity | MODERATE | Proper error handling for each I/O operation |
| Cross-Platform | NEEDS_VERIFY | Process I/O differs on Windows |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | LLM retries tool call |

**Priority Score:** (5 x 3 x 3 x 1) / (15 x 2 + 1) = 45 / 31 = **1.5** -> P3

---

## SWEEP-014 — SQLite errors lack database path context

**Phase:** 2.3 — Error Context

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 0% | Adding context to error messages only |
| Runchanged | 0% | Debugging difficulty, no user impact |
| **RC** | **0** | |
| Agentic Core | NONE | |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | N/A | |
| User Visibility | SILENT | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | N/A | |

**Priority Score:** 0 -> P4
