# Full Sweep 1 — v5.0.1 — 2026-04-11

## Summary

- **Sweep type:** Full — all 10 phases
- **Branch:** `full-sweep-1`
- **Crates scanned:** 24
- **Total findings:** 47
- **Findings with risk matrix:** 47

## Priority Heatmap

| Priority | Count | Agentic Core DIRECT | Agentic Core INDIRECT | Leaf Crate (NONE) |
|----------|-------|---------------------|-----------------------|-------------------|
| **P0 EMERGENCY** | 2 | 2 (SWEEP-701, 702) | 0 | 0 |
| **P1 CRITICAL** | 6 | 2 (SWEEP-001, 016) | 2 (SWEEP-401, 501) | 2 (SWEEP-004, 227) |
| **P2 HIGH** | 17 | 4 | 8 | 5 |
| **P3 MEDIUM** | 13 | 1 | 6 | 6 |
| **P4 LOW** | 9 | 0 | 1 | 8 |

## Top 10 Findings by Priority Score

| Rank | ID | Score | Summary | Phase |
|------|----|-------|---------|-------|
| 1 | SWEEP-701 | **INF** | `file_read` path traversal — arbitrary filesystem read | 7.1 |
| 2 | SWEEP-702 | **INF** | `file_write` path traversal — arbitrary filesystem write | 7.2 |
| 3 | SWEEP-401 | **3500** | `split_message()` UTF-8 byte-slice panic in Telegram/Discord/Slack | 4.1 |
| 4 | SWEEP-501 | **1750** | SQLite main memory backend has no WAL mode or busy_timeout | 5.1 |
| 5 | SWEEP-016 | **933** | Token estimation `len/4` underestimates CJK by 2x — context overflow | 3.1 |
| 6 | SWEEP-004 | **700** | Gemini `&body[..500]` byte-slice panic on error body | 1.2 |
| 7 | SWEEP-227 | **700** | In-process Watchdog is dead code — zero runtime health monitoring | 10.2 |
| 8 | SWEEP-001 | **467** | `eigen_tune.unwrap()` in runtime.rs — indirect guard, not safe | 1.1 |
| 9 | SWEEP-601 | **350** | Anthropic hardcodes `max_tokens: 4096` — truncates long output | 6.1 |
| 10 | SWEEP-018 | **292** | No automatic retry on 429 RateLimited — user must manually retry | 3.4 |

## Critical Patterns (cross-cutting)

### Pattern A: UTF-8 Byte-Slice Panic (8 locations)

Same bug class as the Vietnamese text incident (2026-03-09). Found in:

| File | Line | Code | Phase |
|------|------|------|-------|
| `telegram.rs` | 792-807 | `split_message()` `&remaining[..max_len]` | 4.1 |
| `discord.rs` | 985-993 | `split_message()` `&text[..max_len]` | 4.1 |
| `slack.rs` | 905-928 | `split_message()` `&text[..max_len]` | 4.1 |
| `gemini.rs` | 456 | `&body[..body.len().min(500)]` | 1.2 |
| `shell.rs` | 115 | `content.truncate(MAX_OUTPUT_SIZE)` | 7.4 |
| `file.rs` | 114 | `content.truncate(MAX_OUTPUT_SIZE)` | 7.4 |
| `web_fetch.rs` | 113 | `content.truncate(MAX_OUTPUT_SIZE)` | 7.4 |
| `self_work.rs` | 540 | `safe_msg.truncate(500)` | 7.5 |

**Fix:** Replace all with `char_indices()` safe truncation. Single sweep, zero risk.

### Pattern B: Security Boundaries Not Enforced (3 findings)

| ID | Issue | Blast Radius |
|----|-------|--------------|
| SWEEP-701 | `file_read` allows arbitrary path traversal | SYSTEM |
| SWEEP-702 | `file_write` allows arbitrary path traversal | SYSTEM |
| SWEEP-703 | Shell tool runs `sh -c` with no sandbox | SYSTEM |

### Pattern C: Dead/Unwired Code (1 finding)

| ID | Issue | Impact |
|----|-------|--------|
| SWEEP-227 | In-process Watchdog fully implemented but never instantiated | No runtime health monitoring |

## Artifact Index

| File | Contents |
|------|----------|
| [phase-01-panic.md](phase-01-panic.md) | Panic path audit (9 findings) |
| [phase-02-errors.md](phase-02-errors.md) | Error handling integrity (5 findings) |
| [phase-03-llm.md](phase-03-llm.md) | LLM handling audit (6 findings) |
| [phase-04-channels.md](phase-04-channels.md) | Channel resilience (8 findings) |
| [phase-05-memory.md](phase-05-memory.md) | Memory backend durability (4 findings) |
| [phase-06-providers.md](phase-06-providers.md) | Provider integration (5 findings) |
| [phase-07-tools.md](phase-07-tools.md) | Tool execution safety (8 findings) |
| [phase-08-concurrency.md](phase-08-concurrency.md) | Concurrency & state safety (10 findings) |
| [phase-09-config.md](phase-09-config.md) | Configuration & startup safety (3 findings) |
| [phase-10-watchdog.md](phase-10-watchdog.md) | Watchdog & recovery (7 findings) |
| [fix-plan.md](fix-plan.md) | Ordered fix plan derived from priority scores |

## Decision Gate

**No code changes are authorized until this report is reviewed.**
Every finding has a full 15-dimension risk matrix. Review the matrices before approving any fix.
Agentic Core DIRECT findings require extreme review and full regression test.
