# Phase 1: Panic Path Audit

## Baseline Checks

| Check | Status |
|-------|--------|
| `panic = "unwind"` in `[profile.release]` | PASS |
| Gateway dispatch catch_unwind | PASS (`main.rs:4777,5389`) |
| CLI chat catch_unwind | PASS (`main.rs:7058`) |
| Agent `process_message` catch_unwind | PASS (via gateway) |
| Perpetuum pulse catch_unwind | PASS (`lib.rs:240`) |
| Perpetuum cortex catch_unwind | PASS (`cortex.rs:87`) |
| MCP bridge catch_unwind | **FAIL** — no coverage |
| Hive swarm catch_unwind | **FAIL** — no coverage |
| Cambium deploy catch_unwind | **FAIL** — no coverage |

---

## SWEEP-001 — unwrap() on EigenTune engine in runtime.rs

**Phase:** 1.1 — Unwrap/Expect Scan

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | Single-line change in Agentic Core — logically simple but needs full regression |
| Runchanged | 2% | Guard is correlated but not a direct conditional on `self.eigen_tune` |
| **RC** | **0.18** | Low urgency — guard works today, but fragile to future refactors |
| Agentic Core | **DIRECT** | `crates/temm1e-agent/src/runtime.rs:1593` |
| Blast Radius | CHANNEL | Single chat session affected |
| Reversibility | DEPLOY | Code change, redeploy |
| Data Safety | NONE | No user data involved |
| Concurrency | LOW | Only during EigenTune collection, not every message |
| Provider Coupling | AGNOSTIC | Not provider-specific |
| Test Coverage | NONE | No test for this edge case |
| User Visibility | ERROR | Caught by catch_unwind, user gets error reply |
| Fix Complexity | TRIVIAL | `if let Some(engine) = self.eigen_tune.as_ref()` |
| Cross-Platform | UNIVERSAL | No platform-specific code |
| Incident History | THEORETICAL | Logically correlated guard makes this unlikely |
| Recovery Path | SELF_HEALING | catch_unwind absorbs panic, session continues |

**Priority Score:** (2 x 3 x 5 x 1) / (10 x 1 + 1) = 30 / 11 = **2.7** -> P3 MEDIUM

**Proposed fix:** Replace `.unwrap()` with `if let Some(engine) = self.eigen_tune.as_ref()`.

---

## SWEEP-002 — unwrap() on LLM-generated JSON args in MCP self_add

**Phase:** 1.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Changes MCP tool arg parsing — needs MCP test coverage |
| Runchanged | 5% | LLM can send wrong type for tool args |
| **RC** | **0.31** | |
| Agentic Core | INDIRECT | MCP tool output feeds into agent loop |
| Blast Radius | CHANNEL | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | Only during MCP self-add tool calls |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | ERROR | |
| Fix Complexity | TRIVIAL | `.and_then(\|v\| v.as_str()).ok_or(...)` |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | SELF_HEALING | catch_unwind at process_message level |

**Priority Score:** (5 x 3 x 5 x 1) / (15 x 1 + 1) = 75 / 16 = **4.7** -> P3

**Proposed fix:** Replace `.unwrap()` with `.and_then(|v| v.as_str()).ok_or(Temm1eError::Tool(...))`.

---

## SWEEP-003 — unwrap() on Anima profile fields

**Phase:** 1.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 5% | Simple Option handling change |
| Runchanged | 1% | Guarded by `above_threshold()` which checks `is_some()` |
| **RC** | **0.17** | |
| Agentic Core | INDIRECT | Anima feeds personality data to agent |
| Blast Radius | CHANNEL | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | PARTIAL | |
| User Visibility | ERROR | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | |

**Priority Score:** (1 x 3 x 5 x 1) / (5 x 1 + 1) = 15 / 6 = **2.5** -> P3

---

## SWEEP-004 — Gemini error body byte-slice `&body[..500]`

**Phase:** 1.2 — String Index Slicing

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 5% | Simple char_indices replacement |
| Runchanged | 3% | Gemini error body could contain non-ASCII (model names, Unicode) |
| **RC** | **0.50** | |
| Agentic Core | INDIRECT | Provider error handling feeds back to agent |
| Blast Radius | CHANNEL | Single chat on Gemini provider |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | HIGH | Every failed Gemini request |
| Provider Coupling | SINGLE | Gemini only |
| Test Coverage | NONE | No malformed body test |
| User Visibility | ERROR | Error path panics — user gets catch_unwind error |
| Fix Complexity | TRIVIAL | Use `char_indices()` |
| Cross-Platform | UNIVERSAL | |
| Incident History | HAS_OCCURRED | Same class as Vietnamese text crash |
| Recovery Path | SELF_HEALING | catch_unwind absorbs |

**Priority Score:** (3 x 3 x 5 x 1) / (5 x 1 + 1) = 45 / 6 = **7.5** -> P2

---

## SWEEP-006 — TUI onboarding `&input[..4]` byte-slice

**Phase:** 1.2

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 5% | |
| Runchanged | 1% | API keys are ASCII, but non-ASCII paste is possible |
| **RC** | **0.17** | |
| Agentic Core | NONE | TUI only |
| Blast Radius | ISOLATED | Single TUI session |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | FATAL | TUI crashes |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | RESTART | User restarts TUI |

**Priority Score:** (1 x 1 x 10 x 2) / (5 x 1 + 1) = 20 / 6 = **3.3** -> P3

---

## SWEEP-007 — Config Debug `&k[..4]` byte-slice

**Phase:** 1.2

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 2% | |
| Runchanged | <1% | API keys are ASCII in practice |
| **RC** | **0.33** | |
| Agentic Core | NONE | Config display only |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | ERROR | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | SELF_HEALING | |

**Priority Score:** (1 x 1 x 5 x 1) / (2 x 1 + 1) = 5 / 3 = **1.7** -> P3

---

## SWEEP-008 — No catch_unwind in temm1e-hive

**Phase:** 1.3

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | Adding catch_unwind to worker.run() |
| Runchanged | 5% | Swarm workers execute LLM+tools, panic possible |
| **RC** | **0.45** | |
| Agentic Core | INDIRECT | Hive workers use agent capabilities |
| Blast Radius | ISOLATED | Single hive task |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | Only during swarm operations |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | Task never completes, no error message |
| Fix Complexity | MODERATE | Wrap worker.run() in catch_unwind |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | |
| Recovery Path | NONE | Dead worker permanently busy, task stuck |

**Priority Score:** (5 x 1 x 1 x 10) / (10 x 2 + 1) = 50 / 21 = **2.4** -> P3

---

## SWEEP-009 — No catch_unwind in Cambium deploy

**Phase:** 1.3

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Deploy pipeline has rollback logic that must be preserved |
| Runchanged | 3% | Deploy involves file ops, process management |
| **RC** | **0.19** | |
| Agentic Core | NONE | Cambium is a leaf crate |
| Blast Radius | SYSTEM | Failed deploy can brick installation |
| Reversibility | IRREVERSIBLE | Old binary may be overwritten before panic |
| Data Safety | NONE | |
| Concurrency | NONE | Single deploy at a time |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | PARTIAL | |
| User Visibility | FATAL | Bot goes offline |
| Fix Complexity | MODERATE | Wrap deploy_full() in catch_unwind + rollback on panic |
| Cross-Platform | NEEDS_VERIFY | Process management differs on Windows |
| Incident History | NEVER | |
| Recovery Path | RESTART | Watchdog restarts, but with potentially bricked binary |

**Priority Score:** (3 x 10 x 10 x 2) / (15 x 2 + 1) = 600 / 31 = **19.4** -> P2
