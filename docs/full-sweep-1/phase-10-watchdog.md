# Phase 10: Watchdog & Recovery

## SWEEP-227 — In-process Watchdog is dead code (NEVER instantiated)

**Phase:** 10.2 — In-Process Watchdog

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 25% | Wiring into main.rs, adding periodic health check task |
| Runchanged | 20% | No runtime health monitoring — sustained failures go undetected |
| **RC** | **0.77** | |
| Agentic Core | **DIRECT** | `crates/temm1e-agent/src/watchdog.rs` |
| Blast Radius | GLOBAL | All users — no health aggregation means slow detection |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | LOW | Periodic check only |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | COVERED | Unit tests exist but no integration test |
| User Visibility | SILENT | Failures are handled individually but no systemic detection |
| Fix Complexity | MODERATE | Wire into main.rs, add periodic check task |
| Cross-Platform | UNIVERSAL | |
| Incident History | NEVER | But can't know — health monitoring doesn't exist |
| Recovery Path | NONE | No automated recovery from sustained subsystem failure |

**Priority Score:** (20 x 7 x 1 x 10) / (25 x 2 + 1) = 1400 / 51 = **27.5** -> P1

**Note:** This is a full implementation that was written but never wired in. The code is ready — it just needs to be instantiated in `main.rs` and fed health reports from providers, memory, and channels.

---

## SWEEP-222 — Watchdog binary: /proc check unnecessary on macOS

**Phase:** 10.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 2% | |
| Runchanged | 0% | `kill -0` fallback works correctly |
| **RC** | **0** | |
| Agentic Core | NONE | |
| Blast Radius | ISOLATED | |
| User Visibility | SILENT | |
| Test Coverage | COVERED | |
| Fix Complexity | TRIVIAL | |

**Priority Score:** 0 -> P4

---

## SWEEP-223 — mem::forget(child) leaks Child struct on watchdog restart

**Phase:** 10.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 5% | Replace forget with explicit drop |
| Runchanged | 1% | 500 bytes over 5 restarts |
| **RC** | **0.17** | |
| Agentic Core | NONE | |
| Blast Radius | ISOLATED | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | COVERED | |
| User Visibility | SILENT | |
| Fix Complexity | TRIVIAL | |
| Cross-Platform | NEEDS_VERIFY | Windows may leak process handles |
| Incident History | NEVER | |
| Recovery Path | RESTART | |

**Priority Score:** (1 x 1 x 1 x 2) / (5 x 1 + 1) = 2 / 6 = **0.3** -> P4

---

## SWEEP-224 — Watchdog signal handler uses static mut (UB)

**Phase:** 10.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 15% | Replacing with signal_hook crate |
| Runchanged | 0% | Works in practice, atomic store is signal-safe |
| **RC** | **0** | |
| Agentic Core | NONE | |
| Blast Radius | SYSTEM | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | SILENT | |
| Fix Complexity | MODERATE | Use `signal_hook` crate |
| Cross-Platform | UNIX_ONLY | Windows has no signal handler |
| Incident History | NEVER | |
| Recovery Path | RESTART | |

**Priority Score:** 0 -> P4

---

## SWEEP-225 — Watchdog: Windows has no signal handling

**Phase:** 10.1

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 10% | Add Windows `SetConsoleCtrlHandler` or `ctrlc` crate |
| Runchanged | 100% | Windows watchdog cannot be stopped gracefully |
| **RC** | **9.09** | |
| Agentic Core | NONE | |
| Blast Radius | SYSTEM | |
| Reversibility | DEPLOY | |
| Data Safety | NONE | |
| Concurrency | NONE | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | Must taskkill /F |
| Fix Complexity | MODERATE | |
| Cross-Platform | This IS the cross-platform fix |
| Incident History | NEVER | |
| Recovery Path | MANUAL | |

**Priority Score:** (100 x 10 x 3 x 5) / (10 x 2 + 1) = 15000 / 21 = **714** -> P0 (Windows only)

**Note:** Only affects Windows. macOS and Linux are fine. Marking P3 overall since TEMM1E's primary deployment is Linux/macOS.

---

## SWEEP-234 — Hard-coded 200-message history cap, not configurable

**Phase:** 10.5

| Dimension | Value | Rationale |
|-----------|-------|-----------|
| Rchange | 50% | Changing history cap affects context management and memory usage |
| Runchanged | 20% | Tool-heavy conversations lose early context quickly |
| **RC** | **0.39** | |
| Agentic Core | **DIRECT** | `main.rs:5064` |
| Blast Radius | CHANNEL | Single chat |
| Reversibility | DEPLOY | |
| Data Safety | WRITE | Old messages permanently removed from persistent store |
| Concurrency | HIGH | |
| Provider Coupling | AGNOSTIC | |
| Test Coverage | NONE | |
| User Visibility | DEGRADED | Agent forgets early context |
| Fix Complexity | TRIVIAL | Make configurable via `max_persistent_history` |
| Cross-Platform | UNIVERSAL | |
| Incident History | THEORETICAL | |
| Recovery Path | NONE | Pruned messages are lost |

**Priority Score:** (20 x 3 x 3 x 10) / (50 x 1 + 1) = 1800 / 51 = **35.3** -> P1

---

## Verified PASS Items

| Check | Status | Notes |
|-------|--------|-------|
| Watchdog binary: minimal dependencies | PASS | Zero AI, zero network |
| Restart count window | PASS | Correctly limits to 5 in 300s |
| Circuit breaker state transitions | PASS | Correct (race noted in SWEEP-204) |
| Recovery timeout capped at 5 min | PASS | `MAX_RECOVERY_TIMEOUT` |
| Dead worker detection + message re-dispatch | PASS | Zero message loss |
| Session rollback on panic | PASS | History restored, session usable |
| Mutex poison recovery in circuit breaker | PASS | `unwrap_or_else(\|e\| e.into_inner())` |
