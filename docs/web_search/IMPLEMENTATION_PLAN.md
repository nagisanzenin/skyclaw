# Tem Web Search — Implementation Plan

**Date:** 2026-04-12
**Branch:** `web-search`
**Status:** Plan ready for review, no code yet
**Risk Level:** ZERO (additive only — see `HARMONY_AUDIT.md`)

---

## 1. North star

**One tool. One name. One signature. Free forever by default.**

```rust
web_search(query: string, max_results?: int, backends?: string[]) -> SearchResults
```

The agent's LLM never knows there are multiple backends underneath. It calls `web_search` the same way it calls `shell` or `web_fetch`. Underneath, a dispatcher fans out across selected backends in parallel, merges by score, dedupes by URL, formats for token efficiency, and returns. **No inner LLM call. No classifier round-trip. No paid dependency required.**

Why no inner classifier: per `feedback_llm_as_finite_brain.md`, every LLM call has a token cost. Tem's own agent loop already has the LLM in scope — let *it* pick backends via the optional `backends` parameter, with sensible defaults when omitted. This is one less round-trip per search and gives the LLM real agency.

---

## 2. File layout

A new submodule under `crates/temm1e-tools/src/`:

```
crates/temm1e-tools/src/
  web_search/
    mod.rs              -- WebSearchTool struct + Tool impl + dispatcher entry
    backends/
      mod.rs            -- SearchBackend trait + BackendId enum + registry
      hn.rs             -- HackerNews Algolia              [Phase 1]
      wikipedia.rs      -- Wikipedia REST v1               [Phase 1]
      github.rs         -- GitHub search API               [Phase 1]
      stackexchange.rs  -- Stack Exchange 2.3              [Phase 1]
      reddit.rs         -- Reddit JSON                     [Phase 1]
      marginalia.rs     -- Marginalia public               [Phase 1]
      arxiv.rs          -- arXiv Atom XML                  [Phase 1]
      pubmed.rs         -- PubMed E-utilities              [Phase 1]
      ddg_chrome.rs     -- DDG via stealth Chrome          [Phase 2]
      wikidata.rs       -- Wikidata SPARQL                 [Phase 2]
      searxng.rs        -- Self-hosted SearXNG client      [Phase 3]
      exa.rs            -- Exa neural search (paid)        [Phase 4]
      brave.rs          -- Brave Search API (paid)         [Phase 4]
      tavily.rs         -- Tavily search (paid)            [Phase 4]
    dispatcher.rs       -- Fan-out, merge, dedupe, budget  [Phase 1]
    governor.rs         -- Per-backend rate limiting       [Phase 1]
    types.rs            -- SearchResult, SearchHit, etc.   [Phase 1]
    format.rs           -- LLM-optimized output format     [Phase 1]
    cache.rs            -- LRU result cache (5min TTL)     [Phase 1]
    tests/
      mod.rs            -- Backend integration tests
      mock.rs           -- HTTP mocking helpers
```

**Why a submodule and not a single `web_search.rs`:** PR #42 puts everything in one 308-line file. With 11+ backends, the right shape is a directory. Each backend file is 50-120 lines, all isolated, all independently testable.

---

## 3. Rollout phases (each shippable independently)

### Phase 1 — HTTP-only Tier 1 backends (the v1 ship)

**Scope:** 8 backends, all pure HTTP via reqwest, zero external dependencies beyond what `temm1e-tools` already pulls in.

**Backends:** HackerNews, Wikipedia, GitHub, Stack Exchange, Reddit, Marginalia, arXiv, PubMed.

**What ships:**
1. New `WebSearchTool` registered in `create_tools()` and `create_tools_with_browser()` next to `WebFetchTool`.
2. `SearchBackend` trait + 8 implementations.
3. Dispatcher with parallel fan-out via `tokio::join!` (or `futures::future::join_all` for dynamic N).
4. Per-backend governor with min-interval enforcement (Reddit 10/min, arXiv 3s, PubMed 333ms).
5. In-memory LRU cache with 5-minute TTL.
6. Output format: human-readable but structured for LLM scanning (see `IMPLEMENTATION_DETAILS.md` §5).
7. Unit tests for: format, dedupe, dispatcher, governor, cache. Per-backend tests for request shape (no live network).
8. Integration test that hits HN Algolia (single live test, since it has no rate limit).

**Result:** Tem ships with real, free, multi-backend web search. **80% of agent search queries get high-quality answers with zero setup.**

**Estimated LOC:** ~700 Rust + ~300 tests.

**Compilation gates:** `cargo check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --check`, `cargo test --workspace`. All must pass.

**Self-test:** 10-turn CLI conversation per the protocol in MEMORY.md, with at least 3 turns invoking `web_search` covering tech / factual / discussion queries.

---

### Phase 2 — Browser-backed DDG fallback

**Scope:** Add `ddg_chrome` backend that drives `html.duckduckgo.com/html/` from Tem's existing stealth Chrome.

**Why separate phase:** Reusing `BrowserTool` requires sharing the existing Chrome instance, which is its own integration. Phase 1 ships value without it.

**What ships:**
1. Mechanism to share/borrow a Chrome page from `BrowserTool` for a single navigation + observation.
   - **Option A** (preferred): Add a `borrow_page()` method to `BrowserTool` that returns a tokio mutex guard wrapping a `Page`. Tools that need a page (web_search) acquire, use, drop.
   - **Option B**: Use `BrowserPool` directly with `try_acquire()`.
   - Decision deferred to Phase 2 implementation start, after re-reading `browser.rs` end-to-end.
2. `ddg_chrome.rs` backend: navigate, observe (`browser_observation::ObservationTier::AccessibilityTree`), parse `.result__a` + `.result__snippet`, return.
3. Per-instance 10/min governor.
4. Backend is **only registered when the `browser` Cargo feature is enabled and `ToolsConfig.browser = true`**.
5. Tests: parsing test against a saved DDG HTML fixture.

**Cross-platform:** Already cross-platform — `chromiumoxide` works on Windows/macOS/Linux.

**Estimated LOC:** ~120 backend + ~50 tests + ~60 BrowserTool method.

---

### Phase 3 — SearXNG one-click install

**Scope:** Add the `searxng` backend + a guided install flow.

**What ships:**
1. `searxng.rs` backend that talks to a configured `searxng_url` (from config or `TEMM1E_SEARXNG_URL` env).
2. New CLI command `temm1e search install` that:
   - Detects `docker` in PATH (or `podman`).
   - Writes a baked `~/.temm1e/searxng/settings.yml` with `formats: [html, json]` and a sane engine selection.
   - Runs `docker run -d --name temm1e-searxng -p 8888:8080 -v ~/.temm1e/searxng:/etc/searxng searxng/searxng`.
   - Stores the URL in `~/.temm1e/temm1e.toml` under `[tools.web_search]`.
   - Verifies with a test query.
3. **Auto-prompt:** When the dispatcher sees ALL Tier 1+2 backends fail or rate-limited for a `general_web` query, the next iteration of the agent loop emits a soft suggestion via the channel: *"For unlimited general web search, run `temm1e search install` to set up a local SearXNG (one-time, ~200 MB)."* Suggestion is shown once per session max.
4. Cross-platform: documented as "requires Docker/Podman." Windows users without Docker still have Tiers 1+2.

**Estimated LOC:** ~200 backend + install command + ~80 tests.

---

### Phase 4 — Paid backends (opt-in only)

**Scope:** Add `exa.rs`, `brave.rs`, `tavily.rs`. Each registered ONLY when its env var is set.

**What ships:**
1. Three backend impls, each ~100 lines.
2. Auto-registration in `create_tools()` based on env var presence (the existing PR #42 pattern, applied uniformly).
3. **Brave migration:** the Brave Search MCP entry in `temm1e-mcp/src/self_extend.rs:71` is removed because the native backend supersedes it. (Or kept dormant — see `HARMONY_AUDIT.md` §4 for the decision.)
4. **PR #42 disposition:** Closed as superseded by this branch. The `exa.rs` backend in Phase 4 is the rewritten, tested version of PR #42's logic — credit and prior-art preserved in the file header comment.

**Estimated LOC:** ~300 + ~150 tests.

---

## 4. Public API surface

### 4.1 The Tool (what the agent sees)

Name: `web_search`

Description (the description string the LLM reads):

```
Search the web. Returns a ranked list of results merged from multiple
specialized sources. Works out of the box with no API keys or setup.

Use this when you need current information, documentation, code, discussions,
research papers, or facts that aren't in your training data. The output footer
always tells you which sources are available, which were used, and how to
retry with different parameters if results look thin.

Sources (auto-picked from a sensible default mix; override via `backends`):
  - hackernews:    tech news, opinions, Show HN, Ask HN
  - wikipedia:     facts, definitions, entities, history, biography
  - github:        code, repositories, issues, projects
  - stackoverflow: programming Q&A, error messages, how-tos
  - reddit:        community discussions, opinions, niche subjects
  - marginalia:    blogs, essays, small-web, long-form writing
  - arxiv:         academic papers (CS, math, physics)
  - pubmed:        biomedical and life sciences research
  - duckduckgo:    general web catch-all (Phase 2; requires browser)
  - searxng:       self-hosted unlimited general web (Phase 3; user opt-in)
  - exa:           neural search (Phase 4; paid, set EXA_API_KEY)
  - brave:         Brave Search API (Phase 4; paid, set BRAVE_API_KEY)
  - tavily:        Tavily search (Phase 4; paid, set TAVILY_API_KEY)
  - any custom backend the user has defined in temm1e.toml

When auto results look thin, retry with explicit `backends=[...]`. The
output footer will hint which alternatives are available.

Result size is bounded by three knobs (all clamped to safe hard caps):
  - max_results        (1-30, default 10)   total hits after merge
  - max_total_chars    (1000-16000, default 8000)   hard cap on output bytes
  - max_snippet_chars  (50-500, default 200)        per-hit snippet length
```

Schema:

```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "What to search for. Use natural language. Be specific."
    },
    "max_results": {
      "type": "integer",
      "description": "Total results to return after merging (1-30, default 10).",
      "default": 10
    },
    "max_total_chars": {
      "type": "integer",
      "description": "Hard cap on total output size in characters (1000-16000, default 8000).",
      "default": 8000
    },
    "max_snippet_chars": {
      "type": "integer",
      "description": "Per-hit snippet character cap (50-500, default 200).",
      "default": 200
    },
    "backends": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional. Specific backends to query, e.g. ['hackernews','github'] for a tech query or ['wikipedia'] for a fact. Unknown names are silently ignored. Omit for the default mix. Custom backends defined in config can also be named here."
    },
    "time_range": {
      "type": "string",
      "enum": ["day","week","month","year","all"],
      "description": "Optional. Restrict to recent results. Default: all."
    },
    "category": {
      "type": "string",
      "enum": ["company","research_paper","news","personal_site","financial_report","people","code"],
      "description": "Optional. Category hint. Backends that don't support categories ignore this silently."
    },
    "language": {
      "type": "string",
      "description": "Optional. ISO 639-1 language code, e.g. 'en', 'vi', 'ja'. Backends that don't support language hints ignore this."
    },
    "region": {
      "type": "string",
      "description": "Optional. ISO 3166-1 country code, e.g. 'us', 'vn', 'jp'. Backends that don't support region hints ignore this."
    },
    "include_domains": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional. Only return results from these domains (suffix match). E.g. ['github.com','docs.rs']."
    },
    "exclude_domains": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Optional. Drop results from these domains (suffix match). E.g. ['pinterest.com']."
    },
    "sort": {
      "type": "string",
      "enum": ["relevance","date","score"],
      "description": "Optional. Sort order for merged results. Default: relevance."
    }
  },
  "required": ["query"]
}
```

**Schema notes for the implementer:**
- The `backends` enum is intentionally **not constrained at the schema level** — it's a free-form string array. The dispatcher silently drops unknown names and uses the catalog footer to communicate what's available. This avoids stale enum churn when backends are added/removed and lets custom backends appear without schema regeneration.
- All numeric params are silently clamped to hard caps (`HARD_MAX_*` constants in `types.rs`). Clamping is reported in the footer for transparency.
- All enum strings are lowercase. Case-insensitive parsing is fine but the canonical form is lowercase.

### 4.2 The dispatcher logic (what happens inside)

```
1. Look up cache for (query, backends, time_range) — return if hit.
2. Resolve backend list:
   - If `backends` provided: use those, filter to enabled ones.
   - Else: use default = [wikipedia, hackernews, duckduckgo|github] depending on
     what's enabled. Always include wikipedia (factual baseline) and one of
     {duckduckgo, github} for breadth.
3. For each backend, check governor:
   - If under min-interval, skip + record skip reason.
   - Else, mark in-flight.
4. tokio::join_all(backends.map(|b| b.search(query, max_results).timeout(8s)))
5. For each result:
   - Ok(Ok(hits)) → merge into result set
   - Ok(Err(e))  → log + record backend failure (not fatal)
   - Err(_)      → 8s timeout, record backend timeout (not fatal)
6. Dedupe by normalized URL (strip ?utm_*, lowercase host, drop trailing /).
7. Score = backend_weight * recency_boost * source_signal.
8. Sort, truncate to max_results.
9. Format for output (see §5 of IMPLEMENTATION_DETAILS.md).
10. Cache (5 min TTL).
11. Return ToolOutput { content, is_error: false }
    EVEN IF some backends failed — partial success is success.
    is_error = true ONLY if EVERY backend failed.
```

### 4.3 ToolDeclarations

```rust
ToolDeclarations {
    file_access: vec![],
    network_access: vec![
        "hn.algolia.com".into(),
        "en.wikipedia.org".into(),
        "api.github.com".into(),
        "api.stackexchange.com".into(),
        "www.reddit.com".into(),
        "old.reddit.com".into(),
        "api.marginalia.nu".into(),
        "export.arxiv.org".into(),
        "eutils.ncbi.nlm.nih.gov".into(),
        "html.duckduckgo.com".into(),       // Phase 2
        "query.wikidata.org".into(),         // Phase 2
        "localhost".into(),                  // Phase 3 (SearXNG)
        "api.exa.ai".into(),                 // Phase 4
        "api.search.brave.com".into(),       // Phase 4
        "api.tavily.com".into(),             // Phase 4
    ],
    shell_access: false,
}
```

This is precise per the **`feedback_agi_full_computer_use.md`** principle (Tem gets full computer access, blocked only from things that brick the system) and the security convention (explicit network domains, not `*`).

---

## 5. Concrete integration points

These are the exact files that get touched. Anything not on this list must NOT be modified.

| File | Change |
|---|---|
| `crates/temm1e-tools/src/lib.rs` | Add `mod web_search;` and `pub use web_search::WebSearchTool;`. Add registration block in both `create_tools()` and `create_tools_with_browser()`, gated on `config.http`. |
| `crates/temm1e-tools/src/web_search/**` | NEW directory, all the new code. |
| `crates/temm1e-tools/Cargo.toml` | Add `quick-xml` (for arXiv Atom parsing) IF not already present. Add `lru` (for cache) IF not already present. Verify both. |
| `crates/temm1e-core/src/types/config.rs` | Add an optional `WebSearchConfig` substruct under `ToolsConfig` for tunables (governor windows, cache TTL, default backends). All fields default — no breaking change. |
| `docs/web_search/RESEARCH.md` | (this branch) — already written. |
| `docs/web_search/IMPLEMENTATION_PLAN.md` | (this file) |
| `docs/web_search/IMPLEMENTATION_DETAILS.md` | Per-backend code shapes — next file. |
| `docs/web_search/HARMONY_AUDIT.md` | 100/0 risk sweep — next file. |
| `README.md` | After Phase 1 ships: update tool count, add web_search to the tools list, mention the no-key promise. |
| `CLAUDE.md` | After Phase 1 ships: bump tool count if it appears there. |

**Files we explicitly do NOT touch in Phase 1:**

- `crates/temm1e-mcp/src/self_extend.rs` — Brave entry stays. Decision deferred to Phase 4.
- `crates/temm1e-tools/src/web_fetch.rs` — completely independent, no overlap.
- `crates/temm1e-tools/src/browser.rs` — Phase 2 only.
- `crates/temm1e-tools/src/prowl_blueprints/web_search.md` — the existing browser-based Prowl blueprint stays. It serves a different role (the agent driving Chrome on arbitrary sites), and the new `web_search` tool is a complement, not a replacement.

---

## 6. Configuration

New optional config block (every field defaults — `temm1e.toml` stays valid without it):

```toml
[tools.web_search]
# Defaults for the three size knobs (clamped to hard caps in code)
default_max_results       = 10
default_max_total_chars   = 8000
default_max_snippet_chars = 200

# Per-backend timeout in seconds
backend_timeout_secs = 8

# Cache TTL in seconds (set to 0 to disable)
cache_ttl_secs = 300

# Default backend mix when the agent doesn't pass `backends`
# Phase 1 default: wikipedia + hackernews + github (all free, no key)
# Phase 2 will swap github → duckduckgo (browser-backed general web)
default_backends = ["wikipedia", "hackernews", "github"]

# SearXNG URL (Phase 3) — if set, enables the searxng backend
# searxng_url = "http://localhost:8888"

# Per-backend governors (overrides for advanced users)
[tools.web_search.governors]
reddit         = { interval_ms = 6600 }   # 10/min + 10% buffer
arxiv          = { interval_ms = 3300 }   # ToS 3s + 10% buffer
pubmed         = { interval_ms = 366  }   # 3/sec + 10% buffer
github         = { interval_ms = 6600 }   # 10/min unauth + 10% buffer
marginalia     = { interval_ms = 1100 }   # shared anonymous quota + 10%
stackoverflow  = { interval_ms = 330  }   # ~3/sec + 10%

# Custom HTTP backends — declarative, no Rust required.
# See IMPLEMENTATION_DETAILS.md §4.1 for the full schema.
# [[tools.web_search.custom_backends]]
# id = "kagi"
# description = "Kagi (paid premium search)"
# url_template = "https://kagi.com/api/v0/search?q={query}&limit={max_results}"
# weight = 0.9
# response_path = "data"
# title_field = "title"
# url_field = "url"
# snippet_field = "snippet"
# governor_interval_ms = 1000
# headers = { Authorization = "Bot ${KAGI_API_KEY}" }
```

Defaults are baked into Rust constants so the config is **fully optional**. A user with no `[tools.web_search]` block in their `temm1e.toml` gets sensible defaults and a working tool. Power users can tune everything.

---

## 7. Phasing rationale

The 100/0 rule says ZERO risk before any code. The phasing achieves this by making each phase independently:

- **Additive:** Phase 1 adds files only, modifies `lib.rs` in two well-bounded spots, modifies `Cargo.toml` for at most 2 deps. No existing code changes.
- **Reversible:** Each phase can be rolled back by deleting its files and reverting `lib.rs`.
- **Testable in isolation:** Each backend has its own tests. The dispatcher has its own tests with mock backends. The format function is pure.
- **Behind a feature flag:** Phase 2 (browser-backed) is gated on `#[cfg(feature = "browser")]`. Phases 3 and 4 are runtime-gated by config/env, so they don't affect users who don't opt in.

---

## 8. What we'll know when we're done

After Phase 1, Tem can:

- Answer `"who invented the printing press"` via Wikipedia in <500ms with no setup.
- Answer `"what's the latest on rust async runtimes"` via HN + GitHub in <2s.
- Answer `"how do I parse a CSV in Python with pandas"` via Stack Exchange in <1s.
- Answer `"any cool small-web blogs about SQLite"` via Marginalia in <1s.
- Answer `"recent papers on LLM tool use"` via arXiv in <2s.
- Survive any single backend going down because partial-success is success.
- Cost: $0 forever, with optional paid upgrades for power users.

After Phase 2, also general-web queries via DDG-via-Chrome.

After Phase 3, also unlimited general-web via self-hosted SearXNG (one-click install).

After Phase 4, neural search and premium APIs as opt-in upgrades.

**At every phase, the tool name and signature stay identical.** No agent retraining, no schema migration, no breakage.
