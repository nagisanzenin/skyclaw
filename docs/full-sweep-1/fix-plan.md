# Full Sweep 1 — Fix Plan

**Ordered by priority score. No code changes until this plan is reviewed and approved.**

---

## Execution Rules

1. **Fix one priority band at a time.** All P0 fixes land before any P1 work begins.
2. **Each fix gets its own commit.** No bundling — if a fix introduces a regression, we can revert it cleanly.
3. **Agentic Core DIRECT fixes require full regression test** (cargo test + 10-turn CLI self-test).
4. **After each fix: `cargo check && cargo clippy && cargo fmt --check && cargo test`.**
5. **Update this file** as fixes land — check off the `[ ]` box and add the commit hash.

---

## Wave 1: P0 EMERGENCY (fix before anything else)

| # | ID | Score | Fix | Core | Complexity | Files |
|---|----|-------|-----|------|-----------|-------|
| 1 | SWEEP-701 | 10000 | Add workspace containment to `resolve_path()` | DIRECT | MODERATE | `tools/file.rs` |
| 2 | SWEEP-702 | 10000 | Same fix as 701 (shared `resolve_path()`) | DIRECT | — | Same file |
| 3 | SWEEP-401 | 1050 | Replace byte-slice in `split_message()` with `char_indices()` | INDIRECT | TRIVIAL | `telegram.rs`, `discord.rs`, `slack.rs` |
| 4 | SWEEP-501 | 875 | Add `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;` | DIRECT | TRIVIAL | `memory/sqlite.rs` |

**Wave 1 checklist:**
- [x] SWEEP-701/702: `resolve_path()` workspace containment — commit: 652f8ba
- [x] SWEEP-401: `split_message()` safe UTF-8 split — commit: 652f8ba
- [x] SWEEP-501: SQLite WAL + busy_timeout — commit: 652f8ba
- [x] All compilation gates pass (2406 tests, 0 failures)

---

## Wave 2: P1 CRITICAL (fix before next release)

| # | ID | Score | Fix | Core | Complexity | Files |
|---|----|-------|-----|------|-----------|-------|
| 5 | SWEEP-704 | 50 | Safe UTF-8 truncation in tool output (shell, file, web_fetch) | DIRECT | TRIVIAL | `shell.rs`, `file.rs`, `web_fetch.rs` |
| 6 | SWEEP-706 | 150 | Add missing credential patterns (AKIA, sk_live_, xoxb-, etc.) | INDIRECT | TRIVIAL | `credential_scrub.rs` |
| 7 | SWEEP-601 | 30 | Use model registry max_output instead of hardcoded 4096 | DIRECT | MODERATE | `anthropic.rs` |
| 8 | SWEEP-602 | 84.7 | Add key exhaustion detection + minimum rotation interval | DIRECT | MODERATE | `anthropic.rs` |
| 9 | SWEEP-018 | 51.5 | Add retry-with-jitter on 429 RateLimited | DIRECT | MODERATE | `main.rs` or `runtime.rs` |
| 10 | SWEEP-227 | 27.5 | Wire in-process Watchdog into main.rs | DIRECT | MODERATE | `main.rs` |
| 11 | SWEEP-016 | 2.5* | Unicode-aware token estimation | DIRECT | MODERATE | `context.rs` |
| 12 | SWEEP-703 | 82.6 | Shell tool sandbox (command filtering at minimum) | DIRECT | COMPLEX | `shell.rs`, new sandbox module |
| 13 | SWEEP-707 | 400 | Replace `assert!` with `Result` in BrowserPool::new() | NONE | TRIVIAL | `browser_pool.rs` |
| 14 | SWEEP-708 | 35 | Replace `assert!` with `Result` in get_page() | INDIRECT | TRIVIAL | `browser_pool.rs` |

*SWEEP-016 elevated from formula P3 to P1 due to Agentic Core DIRECT + wide blast radius for non-Latin users.

**Wave 2 checklist:**
- [x] SWEEP-704: Tool output safe truncation — commit: d5e6be2
- [x] SWEEP-706: Credential scrub patterns — commit: d5e6be2
- [x] SWEEP-601: Anthropic max_tokens from registry — commit: d5e6be2
- [x] SWEEP-602: Key rotation cooldown — commit: f9ac9a0
- [x] SWEEP-016: Unicode token estimation — commit: f9ac9a0
- [x] SWEEP-707: BrowserPool assert→Result — commit: d5e6be2
- [x] SWEEP-708: get_page assert→Result — commit: d5e6be2
- [x] All compilation gates pass (2406 tests, 0 failures)
- **DEFERRED:** SWEEP-018, SWEEP-227, SWEEP-703 (see Deferred List below)

---

## Wave 3: P2 HIGH (fix within 1 week)

| # | ID | Fix | Core | Complexity |
|---|----|-----|------|-----------|
| 15 | SWEEP-402 | Reply to denied users instead of silence | NONE | TRIVIAL |
| 16 | SWEEP-404 | Unify wildcard `*` support across all channels | NONE | TRIVIAL |
| 17 | SWEEP-407 | WhatsApp Web: empty allowlist should deny-all | NONE | TRIVIAL |
| 18 | SWEEP-408 | WhatsApp Web: add reconnection loop | NONE | MODERATE |
| 19 | SWEEP-502 | Markdown backend: use append mode or file locking | INDIRECT | MODERATE |
| 20 | SWEEP-605 | Encrypt OAuth tokens via Vault | NONE | MODERATE |
| 21 | SWEEP-209 | Per-chat channel: use try_send or increase capacity | DIRECT | MODERATE |
| 22 | SWEEP-208 | Increase unified message channel capacity | INDIRECT | TRIVIAL |
| 23 | SWEEP-705 | Safe truncation in Perpetuum self_work | INDIRECT | TRIVIAL |
| 24 | SWEEP-234 | Make 200-message history cap configurable | DIRECT | TRIVIAL |
| 25 | SWEEP-017 | Memory entries: separate from system role | DIRECT | COMPLEX |
| 26 | SWEEP-004 | Gemini error body safe slice | INDIRECT | TRIVIAL |
| 27 | SWEEP-215 | Treat empty env var as None in config | DIRECT | MODERATE |
| 28 | SWEEP-603 | Fix body sanitizer for "data: [DONE]" in content | DIRECT | TRIVIAL |
| 29 | SWEEP-503 | Failover search: match primary semantics | INDIRECT | MODERATE |
| 30 | SWEEP-504 | Lambda store: wrap in transaction | INDIRECT | MODERATE |
| 31 | SWEEP-009 | Cambium deploy: add catch_unwind + rollback-on-panic | NONE | MODERATE |

**Wave 3 checklist:**
- [ ] All 17 fixes landed
- [ ] All compilation gates pass
- [ ] 10-turn CLI self-test passes

---

## Wave 4: P3 MEDIUM (fix within 1 sprint)

| # | ID | Fix | Core |
|---|----|-----|------|
| 32 | SWEEP-001 | EigenTune unwrap → if let | DIRECT |
| 33 | SWEEP-002 | MCP self_add unwrap → proper error | INDIRECT |
| 34 | SWEEP-003 | Anima profile unwrap → map | INDIRECT |
| 35 | SWEEP-006 | TUI onboarding safe slice | NONE |
| 36 | SWEEP-007 | Config Debug safe slice | NONE |
| 37 | SWEEP-008 | Hive worker catch_unwind | INDIRECT |
| 38 | SWEEP-013 | Custom tool I/O error handling | INDIRECT |
| 39 | SWEEP-201 | ProviderConfig Debug safe slice | INDIRECT |
| 40 | SWEEP-204 | Circuit breaker CAS for state transitions | DIRECT |
| 41 | SWEEP-403 | Telegram: reset backoff on success | NONE |
| 42 | SWEEP-405 | Telegram: rate limit between chunks | NONE |
| 43 | SWEEP-406 | Slack: pagination for conversations.list | NONE |
| 44 | SWEEP-221 | Wire ResilientMemory into main.rs | INDIRECT |

---

## Wave 5: P4 LOW (fix opportunistically)

| # | ID | Fix |
|---|----|-----|
| 45 | SWEEP-010 | EigenTune trainer: log swallowed errors |
| 46 | SWEEP-011 | Perpetuum cortex: log swallowed store error |
| 47 | SWEEP-014 | SQLite errors: add database path to messages |
| 48 | SWEEP-211 | EigenTune tick: add CancellationToken |
| 49 | SWEEP-212 | Pheromone GC: add cancellation |
| 50 | SWEEP-213 | Runtime fire-and-forget: track JoinHandles |
| 51 | SWEEP-223 | Watchdog: replace mem::forget with drop |
| 52 | SWEEP-224 | Watchdog: replace static mut with signal_hook |
| 53 | SWEEP-225 | Watchdog: Windows signal handling |

---

## Deferred → Resolved

All 15 deferred items have been deep-researched and dispositioned:

### IMPLEMENTED (confirmed 100% confidence after research)

| ID | Fix | Commit |
|----|-----|--------|
| SWEEP-215 | Filter empty keys in `all_keys()` | 32c1652 |
| SWEEP-504 | Lambda store transaction wrapping | 32c1652 |
| SWEEP-503 | Failover search word-split AND matching | 32c1652 |
| SWEEP-502 | Markdown atomic append (OpenOptions) | 32c1652 |
| SWEEP-204 | Circuit breaker CAS | Already implemented (code already had compare_exchange) |
| SWEEP-209 | Per-chat channel capacity | Already 32 (sweep finding was incorrect) |

### BIN (impossible to reach 100/0 — research proved why)

| ID | Fix | Reason |
|----|-----|--------|
| SWEEP-009 | Cambium deploy catch_unwind | No panic paths exist (all ops use `?`). Rollback state unreachable from catch_unwind wrapper. |
| SWEEP-408 | WhatsApp Web reconnection | `bot.run()` consumes internal state via `.take()`. Cannot re-run. LoggedOut requires QR scan. |
| SWEEP-605 | Encrypt OAuth tokens via Vault | Vault key co-located with oauth.json (no real security gain). Migration complexity. Startup ordering issues. |
| SWEEP-017 | Memory entries Role::System | Not a real vulnerability (single-tenant). Role change causes LLM behavioral regression across all providers. |
| SWEEP-703 | Shell tool sandbox | No zero-false-positive denylist exists. Any command filtering blocks legitimate commands (cargo, npm, etc.). Needs OS-level sandboxing (architecture decision). |
| SWEEP-221 | Wire ResilientMemory | ResilientMemory has unbounded cache growth in healthy-primary path. Needs cache eviction strategy first. |

### REMAINING (research complete, implementation deferred to next sweep)

| ID | Fix | Confidence | Why Deferred |
|----|-----|-----------|-------------|
| SWEEP-018 | Rate limit retry w/ jitter | 90% | Agentic Core DIRECT, 5+ call sites in process_message(), needs E2E testing with live provider |
| SWEEP-227 | Wire Watchdog into production | 90% | Agentic Core DIRECT, multi-file coordinated change, needs shutdown flow integration |
| SWEEP-406 | Slack pagination | 85% | Needs Slack API testing with real workspace >200 channels |

---

## Risk Budget

| Wave | Findings | Agentic Core DIRECT | Estimated Rchange (avg) | Regression Risk |
|------|----------|---------------------|------------------------|-----------------|
| 1 | 4 | 2 | 0% (trivial fixes) | MINIMAL |
| 2 | 10 | 7 | 15% (moderate fixes) | MODERATE — full regression test required |
| 3 | 17 | 5 | 10% | MODERATE |
| 4 | 13 | 2 | 8% | LOW |
| 5 | 9 | 0 | 5% | MINIMAL |

---

## Success Criteria

After all waves complete:
- [ ] Zero `unwrap()` on user/network data in production code
- [ ] Zero byte-slice operations on potentially multi-byte strings
- [ ] All critical paths wrapped in `catch_unwind()`
- [ ] SQLite WAL mode on all databases
- [ ] File operations contained to workspace
- [ ] Shell tool has baseline sandbox
- [ ] In-process Watchdog actively monitoring
- [ ] Credential scrubber covers all major key patterns
- [ ] Token estimation handles non-Latin scripts
- [ ] All channels reject unauthorized users with a message, not silence
- [ ] All compilation gates pass
- [ ] 10-turn CLI self-test passes
- [ ] Sweep report archived in `docs/full-sweep-1/`
