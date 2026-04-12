# Tem Web Search — Implementation Details

**Date:** 2026-04-12
**Branch:** `web-search`
**Companion to:** `IMPLEMENTATION_PLAN.md`

This document specifies the exact code shapes, schemas, request/response formats, and edge cases for each backend and the dispatcher. It is the reference the implementer follows when writing the actual Rust.

---

## 1. Core types

```rust
// crates/temm1e-tools/src/web_search/types.rs

use serde::{Deserialize, Serialize};

// =====================================================================
// Constants — defaults and hard caps for output bounding.
// HARD caps cannot be exceeded by any agent param. Defaults can be
// overridden by config or per-call params, both clamped to HARD caps.
// =====================================================================

pub const DEFAULT_MAX_RESULTS:       usize = 10;
pub const DEFAULT_MAX_TOTAL_CHARS:   usize = 8_000;   // ~2K tokens
pub const DEFAULT_MAX_SNIPPET_CHARS: usize = 200;

pub const HARD_MAX_RESULTS:          usize = 30;
pub const HARD_MAX_TOTAL_CHARS:      usize = 16_000;  // ~4K tokens
pub const HARD_MAX_SNIPPET_CHARS:    usize = 500;

pub const MIN_MAX_RESULTS:           usize = 1;
pub const MIN_MAX_TOTAL_CHARS:       usize = 1_000;
pub const MIN_MAX_SNIPPET_CHARS:     usize = 50;

/// Per-backend HTTP body cap (matches PR #42 precedent for safety).
/// Bounds memory pressure during HTTP read, before any parsing.
pub const MAX_BACKEND_RESPONSE_BYTES: usize = 64 * 1024;

/// Multiplier from `max_results` to per-backend raw hit cap.
/// Each backend returns at most `max_results × this` hits before merge.
/// Gives the dispatcher headroom for dedup without one backend dominating.
pub const PER_BACKEND_RAW_MULTIPLIER: usize = 2;

/// A single search hit, normalized across all backends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    /// Result title — always present.
    pub title: String,
    /// Canonical URL — always present.
    pub url: String,
    /// 1-3 line summary or excerpt for the agent to scan.
    pub snippet: String,
    /// Which backend produced this hit (for transparency to the agent).
    pub source: BackendId,
    /// Optional ISO 8601 publish date if the backend exposes one.
    pub published: Option<String>,
    /// Backend-native score in 0..=1 range, used for merging.
    pub score: f32,
    /// Optional structured signals (HN points, SO score, GH stars, etc.).
    /// Rendered into the snippet for human readability if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<HitSignal>,
    /// Other backends that also returned this URL (after dedup).
    /// Empty if this hit was unique. Rendered as "also: github" in the footer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_in: Vec<BackendId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HitSignal {
    HnPoints { points: u32, comments: u32 },
    GithubStars { stars: u32, language: Option<String> },
    StackOverflowScore { score: i32, answers: u32, accepted: bool },
    RedditUpvotes { ups: i32, comments: u32, subreddit: String },
    ArxivAuthors { authors: Vec<String>, primary_category: String },
    PubmedAuthors { authors: Vec<String>, journal: String },
    Wikipedia { description: Option<String> },
    MarginaliaQuality { quality: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendId {
    HackerNews,
    Wikipedia,
    Github,
    StackOverflow,
    Reddit,
    Marginalia,
    Arxiv,
    Pubmed,
    Wikidata,      // Phase 2
    DuckDuckGo,    // Phase 2
    SearXNG,       // Phase 3
    Exa,           // Phase 4
    Brave,         // Phase 4
    Tavily,        // Phase 4
}

impl BackendId {
    /// Stable lower-snake-case name used in tool input/output and config.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HackerNews    => "hackernews",
            Self::Wikipedia     => "wikipedia",
            Self::Github        => "github",
            Self::StackOverflow => "stackoverflow",
            Self::Reddit        => "reddit",
            Self::Marginalia    => "marginalia",
            Self::Arxiv         => "arxiv",
            Self::Pubmed        => "pubmed",
            Self::Wikidata      => "wikidata",
            Self::DuckDuckGo    => "duckduckgo",
            Self::SearXNG       => "searxng",
            Self::Exa           => "exa",
            Self::Brave         => "brave",
            Self::Tavily        => "tavily",
        }
    }
}

/// Resolved search request after clamping. All caps already enforced.
/// This is what backends actually see. Agent input → resolve() → SearchRequest.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    /// Final result count after merge (clamped to HARD_MAX_RESULTS).
    pub max_results: usize,
    /// Hard cap on total format output bytes (clamped to HARD_MAX_TOTAL_CHARS).
    pub max_total_chars: usize,
    /// Per-hit snippet character cap (clamped to HARD_MAX_SNIPPET_CHARS).
    pub max_snippet_chars: usize,
    pub time_range: TimeRange,
    pub category: Option<Category>,
    pub language: Option<String>,
    pub region: Option<String>,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
    pub sort: SortOrder,
}

impl SearchRequest {
    /// Per-backend raw hit cap. Bounds memory pressure during dispatch.
    pub fn per_backend_raw_cap(&self) -> usize {
        self.max_results.saturating_mul(PER_BACKEND_RAW_MULTIPLIER)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRange { Day, Week, Month, Year, All }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder { Relevance, Date, Score }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Company,
    ResearchPaper,
    News,
    PersonalSite,
    FinancialReport,
    People,
    Code,
}

/// Raw user-supplied tool input — pre-clamping. Parsed from JSON.
/// Numeric fields are Option so we can distinguish "not provided" from "0".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RawSearchInput {
    pub query: String,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default)]
    pub max_total_chars: Option<usize>,
    #[serde(default)]
    pub max_snippet_chars: Option<usize>,
    #[serde(default)]
    pub backends: Option<Vec<String>>,
    #[serde(default)]
    pub time_range: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub include_domains: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_domains: Option<Vec<String>>,
    #[serde(default)]
    pub sort: Option<String>,
}

/// Outcome of clamping the raw input. Includes a list of clamps applied so
/// the dispatcher can surface them in the footer (transparency for the agent).
#[derive(Debug, Clone)]
pub struct ResolvedInput {
    pub req: SearchRequest,
    pub backends_filter: Option<Vec<BackendId>>,
    pub clamps_applied: Vec<String>,
}

#[derive(Debug)]
pub enum BackendError {
    /// Network/transport error.
    Network(String),
    /// Backend returned a non-success status.
    Http { status: u16, body: String },
    /// Backend returned malformed data.
    Parse(String),
    /// Skipped due to rate limit governor.
    RateLimited { retry_after_ms: u64 },
    /// Backend timed out (8s default).
    Timeout,
    /// Backend not configured/enabled (e.g., paid backend with no key).
    Disabled,
}
```

---

## 2. The SearchBackend trait

```rust
// crates/temm1e-tools/src/web_search/backends/mod.rs

use async_trait::async_trait;

#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Stable backend identifier.
    fn id(&self) -> BackendId;

    /// Whether this backend is enabled (configured/has key/etc.).
    /// Disabled backends are skipped silently.
    fn enabled(&self) -> bool;

    /// Default weight in the merge step. Higher = more trusted for general queries.
    /// Per-query weight may differ — see scoring rules in dispatcher.rs.
    fn default_weight(&self) -> f32;

    /// Run a search. Must enforce its own per-backend timeout? No — the
    /// dispatcher wraps each call in tokio::time::timeout(8s). Backends should
    /// only handle backend-specific errors.
    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError>;
}
```

---

## 3. Per-backend specs

### 3.1 HackerNews Algolia (`backends/hn.rs`)

```rust
pub struct HackerNewsBackend {
    client: reqwest::Client,
}

impl HackerNewsBackend {
    pub fn new(client: reqwest::Client) -> Self { Self { client } }
}

#[async_trait]
impl SearchBackend for HackerNewsBackend {
    fn id(&self) -> BackendId { BackendId::HackerNews }
    fn enabled(&self) -> bool { true }  // always free
    fn default_weight(&self) -> f32 { 1.0 }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        // URL: https://hn.algolia.com/api/v1/search
        // Params:
        //   query        = req.query
        //   tags         = "story"
        //   hitsPerPage  = req.max_results.min(50)
        //   numericFilters = "created_at_i>{cutoff}"  if req.time_range != All
        //
        // Sort by relevance for `All` and `Year`; switch to /search_by_date
        // when time_range is Day or Week (recency dominates relevance).
        //
        // Response: { hits: [{ title, url, points, num_comments, author, created_at_i, story_text }] }
        //
        // For each hit:
        //   title:    hit.title
        //   url:      hit.url OR hit.story_url OR `https://news.ycombinator.com/item?id={objectID}`
        //   snippet:  hit.story_text (truncated to 200 chars) OR ""
        //   score:    f32::min(1.0, hit.points / 500.0)
        //   signal:   HnPoints { points, comments: num_comments }
        //   published: ISO 8601 from created_at_i
        todo!()
    }
}
```

**Edge cases:**

- `hit.url` may be null for Show HN posts that link to the HN thread itself — use `https://news.ycombinator.com/item?id={objectID}` as fallback.
- `hit.title` may include `[pdf]` or `[video]` markers — preserve them.
- `created_at_i` is Unix seconds — convert to ISO 8601 with `chrono::DateTime::from_timestamp`.

**No governor needed** — Algolia's infrastructure absorbs unauth load.

---

### 3.2 Wikipedia REST (`backends/wikipedia.rs`)

```rust
pub struct WikipediaBackend {
    client: reqwest::Client,
    user_agent: String,  // "Tem/x.y.z (https://github.com/temm1e-labs/temm1e)"
}
```

**Endpoint:** `GET https://en.wikipedia.org/w/rest.php/v1/search/page?q=Q&limit=N`

**Required header:** `User-Agent: Tem/{VERSION} (https://github.com/temm1e-labs/temm1e)` — Wikimedia's bot policy requires a descriptive UA.

**Response → SearchHit mapping:**
```
title:     pages[i].title
url:       https://en.wikipedia.org/wiki/{pages[i].key}
snippet:   strip HTML from pages[i].excerpt + " — " + pages[i].description
score:     1.0 - (i / max_results)  // Wikipedia returns by relevance, no score
signal:    Wikipedia { description: pages[i].description }
published: None
```

**Excerpt cleanup:** Wikipedia returns `<span class="searchmatch">...</span>` markers in excerpts. Strip with a single regex `<[^>]+>` or use `html_escape::decode_html_entities` if already in tree.

**Multilingual:** Phase 1 hardcodes `en.wikipedia.org`. Phase 5 (post-launch) could detect user locale and route to `{lang}.wikipedia.org`.

**No governor needed** — Wikimedia's "reasonable use" tolerates ~200 req/sec/IP.

---

### 3.3 GitHub (`backends/github.rs`)

```rust
pub struct GithubBackend {
    client: reqwest::Client,
    user_agent: String,
    optional_token: Option<String>,  // Reads GITHUB_TOKEN env if set
}
```

**Endpoint:** `GET https://api.github.com/search/repositories?q={query}&per_page={N}`

**Headers:**
- `User-Agent: Tem/{VERSION}` (mandatory — GitHub rejects requests without)
- `Accept: application/vnd.github+json`
- `X-GitHub-Api-Version: 2022-11-28`
- `Authorization: Bearer {token}` IF `GITHUB_TOKEN` is set (raises rate limit 10→30 req/min)

**Multi-endpoint dispatch:** A single call defaults to `/search/repositories`. If the query contains code-like tokens (heuristic: contains `()`, `::`, `{`, `<`, `=>`, or quoted strings), additionally hit `/search/code`. Both run in parallel via `tokio::join!`. **No LLM call** — the heuristic is local (and is the one place we accept a tiny keyword check, justified because GH has separate physical endpoints, not semantic decisions). If `feedback_no_keyword_matching.md` is read strictly, replace with: always query both, dedupe by URL.

→ **Decision:** Always query both `/repositories` and `/code`, dedupe by URL. Two HTTP calls per GitHub backend invocation. Removes the keyword check entirely. ✅

**Response → SearchHit mapping (`/repositories`):**
```
title:     items[i].full_name
url:       items[i].html_url
snippet:   items[i].description (truncate 200 chars)
score:     log10(stargazers_count + 1) / 6.0   (clamped 0..=1; 1M stars ≈ 1.0)
signal:    GithubStars { stars: stargazers_count, language: items[i].language }
published: items[i].pushed_at
```

**Response → SearchHit mapping (`/code`):**
```
title:     items[i].name + " in " + items[i].repository.full_name
url:       items[i].html_url
snippet:   items[i].path  (no excerpt available without a second call — skip)
score:     0.5  (uniform, since /code doesn't expose stars)
signal:    None
published: None
```

**Governor:** 6000ms min interval (10/min unauth). With token: 2000ms (30/min).

---

### 3.4 Stack Exchange (`backends/stackexchange.rs`)

**Endpoint:** `GET https://api.stackexchange.com/2.3/search/advanced?q={query}&site=stackoverflow&pagesize={N}&order=desc&sort=relevance`

**No headers required** beyond `User-Agent`.

**Response shape:** `{ items: [{ question_id, title, link, score, answer_count, is_answered, accepted_answer_id, tags, last_activity_date }] }`

**Cross-site:** Phase 1 hardcodes `site=stackoverflow`. Phase 5: route to `serverfault` for sysadmin queries, `superuser` for desktop, `math` for math (LLM-side hint via `time_range`-style enum, not keyword matching).

**Mapping:**
```
title:     items[i].title (HTML-decoded)
url:       items[i].link
snippet:   "{score} score · {answer_count} answers · tags: {tags.join(\", \")}"
           plus "(accepted ✓)" if accepted_answer_id present
score:     f32::min(1.0, items[i].score as f32 / 200.0)
signal:    StackOverflowScore { score, answers: answer_count, accepted: accepted_answer_id.is_some() }
published: from last_activity_date (Unix → ISO 8601)
```

**Governor:** 300ms min interval (≈300/day budget spread across 24h, but we don't enforce daily — just per-call interval). Free tier hard cap is 300/day per IP. Worth surfacing in error messages: "Stack Exchange daily quota reached, will reset at midnight UTC."

---

### 3.5 Reddit (`backends/reddit.rs`)

**Endpoint:** `GET https://old.reddit.com/search.json?q={query}&limit={N}&raw_json=1&sort=relevance`

`old.reddit.com` is preferred over `www.reddit.com` because the JSON shape is leaner and the rate limiter is the same.

**Required header:** `User-Agent: Tem/{VERSION} (by /u/temm1e_labs)` — Reddit blocks generic UAs.

**Response shape:** `{ data: { children: [{ data: { title, selftext, url, permalink, score, num_comments, subreddit, created_utc, author, is_self, link_flair_text } }] } }`

**Mapping:**
```
title:     data.title
url:       if data.is_self: "https://reddit.com{data.permalink}" else data.url
snippet:   data.selftext.truncate(200) OR ("Link post in r/" + subreddit)
score:     f32::min(1.0, data.score as f32 / 1000.0)
signal:    RedditUpvotes { ups: score, comments: num_comments, subreddit }
published: from created_utc (Unix → ISO 8601)
```

**Governor:** 6000ms min interval (10/min unauth, hard limit).

---

### 3.6 Marginalia (`backends/marginalia.rs`)

**Endpoint:** `GET https://api.marginalia.nu/public/search/{url-encoded-query}?count={N}`

The literal string `public` is the API key, embedded in the URL path.

**Response shape:** `{ query, license, page, pages, results: [{ url, title, description, quality, dataset, format }] }`

**Mapping:**
```
title:     results[i].title
url:       results[i].url
snippet:   results[i].description.truncate(200)
score:     results[i].quality (already 0..=1 ish, clamp)
signal:    MarginaliaQuality { quality }
published: None
```

**Governor:** 1000ms min interval. Marginalia's shared anonymous quota is global, so we should not hammer it.

**Failure mode:** Returns 503 when shared quota is saturated. Surface as `BackendError::RateLimited { retry_after_ms: 60_000 }` and continue.

---

### 3.7 arXiv (`backends/arxiv.rs`)

**Endpoint:** `GET https://export.arxiv.org/api/query?search_query=all:{query}&start=0&max_results={N}&sortBy=relevance&sortOrder=descending`

**Response:** Atom XML. Parse with `quick-xml`.

**Required structure to extract per `<entry>`:**
```xml
<entry>
  <id>http://arxiv.org/abs/2501.12345v1</id>
  <title>Paper title</title>
  <summary>Abstract...</summary>
  <author><name>...</name></author>
  <published>2026-01-15T00:00:00Z</published>
  <link rel="alternate" href="..." />
  <arxiv:primary_category term="cs.AI" />
</entry>
```

**Mapping:**
```
title:     entry.title.trim().replace("\n", " ")
url:       entry.id  (always abs URL)
snippet:   entry.summary.truncate(250).replace("\n", " ")
score:     1.0 - (i / max_results)  (arXiv doesn't expose relevance scores)
signal:    ArxivAuthors { authors, primary_category }
published: entry.published
```

**Retry policy:** arXiv's API is single-connection, 3-second-spacing. Add a single retry on 503 with 5s backoff. If the second attempt also fails, return `BackendError::Network` and continue.

**Governor:** 3000ms min interval (per arXiv ToS).

---

### 3.8 PubMed (`backends/pubmed.rs`)

**Two-step protocol:**

Step 1 — `esearch`:
```
GET https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi
    ?db=pubmed&term={query}&retmode=json&retmax={N}
```
Returns: `{ esearchresult: { idlist: ["12345", "67890", ...], count, ... } }`

Step 2 — `esummary` (single batch call for all UIDs):
```
GET https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi
    ?db=pubmed&id=12345,67890,...&retmode=json
```
Returns: `{ result: { uids: [...], "12345": { title, authors, source, pubdate, articleids, ... } } }`

**Mapping per UID:**
```
title:     result[uid].title
url:       https://pubmed.ncbi.nlm.nih.gov/{uid}/
snippet:   result[uid].authors[0..3].joined(", ") + " · " + result[uid].source + " · " + result[uid].pubdate
score:     1.0 - (i / max_results)
signal:    PubmedAuthors { authors, journal: result[uid].source }
published: parse pubdate (format "2026 Jan 15" → ISO 8601)
```

**Governor:** 333ms min interval (3 req/sec without key). Two HTTP calls per search but only one governor tick because they're sequential and complete in <1s.

---

### 3.9 DuckDuckGo via stealth Chrome (`backends/ddg_chrome.rs`) — Phase 2

**Dependencies:** Requires the `browser` Cargo feature. Enabled at construction time only if `BrowserTool` is available.

**Flow:**
```
1. Acquire a Chrome page from BrowserTool (mechanism TBD in Phase 2 kickoff).
2. page.goto(format!("https://html.duckduckgo.com/html/?q={url-encoded-query}"))
3. page.wait_for_navigation(NetworkIdle, 5s)
4. page.evaluate(`Array.from(document.querySelectorAll('.result')).slice(0, N).map(r => ({
       title: r.querySelector('.result__a')?.innerText,
       url:   r.querySelector('.result__a')?.href,
       snippet: r.querySelector('.result__snippet')?.innerText
   }))`)
5. Parse the JS result → Vec<SearchHit>
6. Release the page back to BrowserTool.
```

**Mapping:**
```
title:     js_result[i].title
url:       js_result[i].url  (note: DDG wraps URLs in /l/?uddg=... — must unwrap)
snippet:   js_result[i].snippet.truncate(200)
score:     1.0 - (i / max_results)
signal:    None
published: None
```

**URL unwrapping:** DDG redirects link through `/l/?uddg={base64-url-encoded-target}`. Strip the wrapper to get the real URL.

**Governor:** 6000ms min interval (10/min). Strict because we don't want to be the agent that gets DDG to ban Chrome UAs.

---

### 3.10 Wikidata SPARQL (`backends/wikidata.rs`) — Phase 2

**Endpoint:** `GET https://query.wikidata.org/sparql?query={url-encoded-sparql}&format=json`

**Templates:** Phase 2 ships 3 templates only:
1. `entity_search` — given a label, return Q-IDs and descriptions (uses `wbsearchentities` API actually, simpler than SPARQL).
2. `instance_of` — given a Q-ID and a type, list all instances.
3. `properties_of` — given a Q-ID, list its properties.

**Implementation note:** Wikidata is the most complex backend because SPARQL generation is hard. **For Phase 2 we ship only template-driven queries**, not free-form SPARQL. Free-form SPARQL is a Phase 5+ stretch.

---

### 3.11 SearXNG (`backends/searxng.rs`) — Phase 3

**Endpoint:** `GET {searxng_url}/search?q={query}&format=json&engines={engines}`

`searxng_url` from config. `engines` from config (default: `google,bing,duckduckgo`).

**Response shape:** `{ query, results: [{ url, title, content, engine, score, publishedDate }], suggestions, infoboxes }`

**Mapping:**
```
title:     results[i].title
url:       results[i].url
snippet:   results[i].content.truncate(200)
score:     results[i].score (already normalized) OR 1.0 - i/N
signal:    None
published: results[i].publishedDate
```

**Governor:** None — local SearXNG has no rate limit.

---

### 3.12 Paid backends (Phase 4)

**Exa** (`backends/exa.rs`) — port from PR #42's `web_search.rs`. The dispatch logic and JSON shape are well-documented in PR #42; the changes are: implement `SearchBackend` trait, return `Vec<SearchHit>`, add `enabled() = std::env::var("EXA_API_KEY").is_ok()`.

**Brave** (`backends/brave.rs`) — `GET https://api.search.brave.com/res/v1/web/search?q={query}&count={N}` with `X-Subscription-Token: {key}` header. Response → standard mapping.

**Tavily** (`backends/tavily.rs`) — `POST https://api.tavily.com/search` with body `{api_key, query, max_results, search_depth: "basic"}`. Response → standard mapping.

All three: `enabled()` returns true only if the env var is set.

---

## 4. Dispatcher (`web_search/dispatcher.rs`)

```rust
pub struct Dispatcher {
    backends: Vec<Arc<dyn SearchBackend>>,
    governor: Arc<Governor>,
    cache: Arc<Cache>,
    config: WebSearchConfig,
}

impl Dispatcher {
    /// Resolve raw user input into a clamped SearchRequest. All caps enforced.
    /// Returns the resolved request, optional backend filter, and a list of
    /// clamps applied for footer transparency.
    pub fn resolve(&self, raw: RawSearchInput) -> Result<ResolvedInput, BackendError> {
        let mut clamps = Vec::new();

        // Required field
        let query = raw.query.trim().to_string();
        if query.is_empty() {
            return Err(BackendError::Parse("query must be non-empty".into()));
        }

        // Numeric clamps with transparency
        let max_results = clamp_with_log(
            raw.max_results.unwrap_or(self.config.default_max_results),
            MIN_MAX_RESULTS, HARD_MAX_RESULTS, "max_results", &mut clamps,
        );
        let max_total_chars = clamp_with_log(
            raw.max_total_chars.unwrap_or(self.config.default_max_total_chars),
            MIN_MAX_TOTAL_CHARS, HARD_MAX_TOTAL_CHARS, "max_total_chars", &mut clamps,
        );
        let max_snippet_chars = clamp_with_log(
            raw.max_snippet_chars.unwrap_or(self.config.default_max_snippet_chars),
            MIN_MAX_SNIPPET_CHARS, HARD_MAX_SNIPPET_CHARS, "max_snippet_chars", &mut clamps,
        );

        // Enum parses (silently default to safe values if invalid)
        let time_range = raw.time_range.as_deref().and_then(parse_time_range)
            .unwrap_or(TimeRange::All);
        let sort = raw.sort.as_deref().and_then(parse_sort)
            .unwrap_or(SortOrder::Relevance);
        let category = raw.category.as_deref().and_then(parse_category);

        // Backend filter — translate names to BackendId, drop unknowns silently
        let backends_filter = raw.backends.map(|names| {
            names.iter().filter_map(|n| BackendId::from_str(n.as_str())).collect()
        });

        let req = SearchRequest {
            query,
            max_results,
            max_total_chars,
            max_snippet_chars,
            time_range,
            category,
            language: raw.language,
            region: raw.region,
            include_domains: raw.include_domains.unwrap_or_default(),
            exclude_domains: raw.exclude_domains.unwrap_or_default(),
            sort,
        };

        Ok(ResolvedInput { req, backends_filter, clamps_applied: clamps })
    }

    pub async fn search(&self, raw: RawSearchInput) -> DispatcherOutput {
        // 1. Resolve + clamp
        let resolved = match self.resolve(raw) {
            Ok(r) => r,
            Err(e) => return DispatcherOutput::input_error(e),
        };
        let req = &resolved.req;

        // 2. Cache lookup
        let cache_key = CacheKey::from_request(req, &resolved.backends_filter);
        if let Some(cached) = self.cache.get(&cache_key) {
            return cached;
        }

        // 3. Resolve backend list (built-in + custom)
        let selected: Vec<Arc<dyn SearchBackend>> = match &resolved.backends_filter {
            Some(ids) => self.backends.iter()
                .filter(|b| ids.contains(&b.id()) && b.enabled())
                .cloned().collect(),
            None => self.default_set(),
        };

        if selected.is_empty() {
            return DispatcherOutput::all_disabled(self.catalog(), resolved.clamps_applied);
        }

        // 4. Governor + parallel fan-out
        let futures = selected.iter().map(|b| {
            let governor = self.governor.clone();
            let backend = b.clone();
            let req = req.clone();
            let cap = req.per_backend_raw_cap();
            async move {
                if let Err(retry_after) = governor.try_acquire(backend.id()).await {
                    return BackendOutcome::Skipped {
                        id: backend.id(),
                        reason: format!("rate limit, retry in {}ms", retry_after),
                    };
                }
                let started = std::time::Instant::now();
                let result = tokio::time::timeout(
                    self.config.backend_timeout(),
                    backend.search(&req),
                ).await;
                match result {
                    Ok(Ok(mut hits)) => {
                        // Per-backend raw cap to prevent one backend dominating
                        if hits.len() > cap { hits.truncate(cap); }
                        BackendOutcome::Ok { id: backend.id(), hits, latency: started.elapsed() }
                    }
                    Ok(Err(e)) => BackendOutcome::Failed { id: backend.id(), error: format!("{:?}", e) },
                    Err(_) => BackendOutcome::Timeout { id: backend.id() },
                }
            }
        });

        let outcomes = futures::future::join_all(futures).await;

        // 5. Merge + dedupe + score + sort + truncate to max_results
        let merged = merge_and_score(outcomes, req, &self.backends);

        // 6. Cache
        self.cache.put(cache_key, merged.clone());

        merged
    }

    /// Default backend set. Always includes wikipedia (factual baseline)
    /// + hackernews (tech baseline) + one general-web source (github for
    /// Phase 1, duckduckgo when Phase 2 lands).
    fn default_set(&self) -> Vec<Arc<dyn SearchBackend>> {
        let preferred: Vec<BackendId> = self.config.default_backends.iter()
            .filter_map(|s| BackendId::from_str(s.as_str()))
            .collect();
        let mut chosen: Vec<Arc<dyn SearchBackend>> = self.backends.iter()
            .filter(|b| preferred.contains(&b.id()) && b.enabled())
            .cloned().collect();
        if chosen.is_empty() {
            chosen = self.backends.iter().filter(|b| b.enabled()).take(3).cloned().collect();
        }
        chosen
    }

    /// Catalog of all backends, partitioned by enabled state.
    /// Used by the format footer for agent discoverability.
    fn catalog(&self) -> Catalog {
        let mut available = Vec::new();
        let mut disabled_with_hint = Vec::new();
        let mut custom = Vec::new();
        for b in &self.backends {
            if b.enabled() {
                if b.is_custom() { custom.push(b.id()); }
                else { available.push(b.id()); }
            } else if let Some(env) = b.disabled_env_hint() {
                disabled_with_hint.push((b.id(), env));
            }
        }
        Catalog { available, disabled_with_hint, custom }
    }
}

fn clamp_with_log(value: usize, min: usize, max: usize, name: &str, clamps: &mut Vec<String>) -> usize {
    if value < min {
        clamps.push(format!("{} {} → {}", name, value, min));
        min
    } else if value > max {
        clamps.push(format!("{} {} → {}", name, value, max));
        max
    } else {
        value
    }
}
```

### 4.1 Custom backends (declarative HTTP, no Rust required)

Power users plug in additional HTTP search APIs via `temm1e.toml` config without writing code:

```toml
[[tools.web_search.custom_backends]]
id = "kagi"
description = "Kagi (paid premium search)"
url_template = "https://kagi.com/api/v0/search?q={query}&limit={max_results}"
method = "GET"
weight = 0.9
governor_interval_ms = 1000

[tools.web_search.custom_backends.headers]
Authorization = "Bot ${KAGI_API_KEY}"

# Where the result array lives in the JSON response (dot-path)
response_path = "data"
title_field = "title"
url_field = "url"
snippet_field = "snippet"
score_field = "score"   # optional; defaults to inverse-index ranking
```

Custom backends are loaded at tool construction time. Each becomes a `CustomHttpBackend` that implements `SearchBackend`. They show up in the catalog under `custom:` and can be requested by the agent via `backends=["kagi"]`. Template substitution: `{query}`, `{max_results}`, and any `${ENV_VAR}` in headers.

**Validation at config load:**
- `url_template` must contain `{query}`.
- `response_path`, `title_field`, `url_field` must be non-empty.
- Unknown ${ENV_VAR} → backend is loaded but `enabled()` returns false until env var present.
- `weight` clamped to `0.0..=1.0`.
- `governor_interval_ms` minimum 100ms (no abuse).

### 4.1 Merging algorithm

```
1. Collect all hits from all successful backends into a flat Vec<SearchHit>.
2. Normalize URLs (strip fragments, lowercase host, drop trailing /,
   strip ?utm_*, ?ref=*, ?fbclid=*).
3. Group by normalized URL. For duplicates:
   - Keep the hit with the highest source confidence.
   - Append the additional source(s) to a `also_in: Vec<BackendId>` field
     (so the LLM sees "also from: github" etc.).
4. Compute final score = backend_default_weight * normalized_backend_score.
5. Apply tiebreakers: HN points > GH stars > SO score > recency.
6. Sort descending by final score.
7. Truncate to req.max_results.
```

### 4.2 DispatcherOutput → ToolOutput

```rust
pub struct DispatcherOutput {
    pub hits: Vec<SearchHit>,
    pub total_candidates_before_truncation: usize,
    pub backends_succeeded: Vec<BackendId>,
    pub backends_failed: Vec<(BackendId, String)>,
    pub backends_skipped: Vec<(BackendId, String)>,
    pub catalog: Catalog,
    pub clamps_applied: Vec<String>,
    pub query: String,
    pub req: SearchRequest,
}

pub struct Catalog {
    pub available: Vec<BackendId>,                  // enabled built-in
    pub disabled_with_hint: Vec<(BackendId, String)>, // not enabled, env var hint
    pub custom: Vec<BackendId>,                     // enabled custom
}

impl DispatcherOutput {
    pub fn into_tool_output(self) -> ToolOutput {
        let content = format::render(&self);
        // SUCCESS even if some backends failed.
        // Failure ONLY if every backend failed AND zero hits AND no clamps to report.
        let is_error = self.hits.is_empty()
            && self.backends_succeeded.is_empty()
            && self.clamps_applied.is_empty();
        ToolOutput { content, is_error }
    }
}
```

### 4.3 Merging algorithm (updated for new params)

```
1. Collect all hits from all successful backends into a flat Vec<SearchHit>.
2. Apply include_domains / exclude_domains filter (if non-empty).
3. Normalize URLs (strip fragments, lowercase host, drop trailing /,
   strip ?utm_*, ?ref=*, ?fbclid=*).
4. Group by normalized URL. For duplicates:
   - Keep the hit with the highest source confidence.
   - Append the additional source(s) to `also_in: Vec<BackendId>`.
5. Compute final score = backend_default_weight * normalized_backend_score.
6. Apply sort:
   - Relevance (default): by final score
   - Date: by published descending (None last)
   - Score: by raw backend score (no weight multiplier)
7. Apply tiebreakers: HN points > GH stars > SO score > recency.
8. Truncate to req.max_results (record total_candidates_before_truncation).
9. Truncate each hit's snippet to req.max_snippet_chars (UTF-8 safe).
```

Domain filtering rule: a hit is kept if `include_domains` is empty OR the host matches any include suffix; AND if no exclude_domains entry matches the host as a suffix. Suffix matching enables `github.com` to match `docs.github.com`.

---

## 5. Output format (LLM-optimized)

The format function is the highest-leverage piece. Bad format = wasted tokens + agent confusion. The design principle: **scannable like a list, structured like data, concise like a tweet, transparent about limits.**

### 5.1 Example output (full)

```
Search results for: "rust async runtime benchmarks"
10 of 27 found · 7,842 / 8,000 chars · used: hackernews, github, wikipedia

[1] tokio vs async-std vs smol — 2026 benchmarks
    https://github.com/matklad/async-bench
    ★ 2,847 · Rust · A maintained microbenchmark suite that compares
    tokio, async-std, smol, and glommio across CPU and IO workloads.
    source: github · also: hackernews

[2] Why I switched from async-std to tokio
    https://news.ycombinator.com/item?id=35421023
    ↑ 412 points · 207 comments · 2026-03-04
    "After two years of running production services on async-std, I
    moved everything to tokio. Here's what I learned about ecosystem..."
    source: hackernews

[3] Tokio (software)
    https://en.wikipedia.org/wiki/Tokio_(software)
    Asynchronous runtime for the Rust programming language.
    Provides building blocks for writing network applications.
    source: wikipedia

... (7 more)

─────
Used:        hackernews, github, wikipedia
Available:   hackernews, wikipedia, github, stackoverflow, reddit, marginalia, arxiv, pubmed
Not enabled: exa (set EXA_API_KEY), brave (set BRAVE_API_KEY), tavily (set TAVILY_API_KEY)
Skipped:     reddit (rate limit, retry in 4s)
Truncated:   17 hits dropped (over budget) — try max_results=20 or max_total_chars=16000
```

### 5.2 Format rules

**Header block (always present):**
- Line 1: `Search results for: "{query}"`
- Line 2: status line — `{shown} of {total} found · {used_chars} / {budget_chars} chars · used: {backend_list}`

**Per hit (sorted by score, top first):**
- `[N]` index then title on the same line.
- URL indented 4 spaces on the next line.
- Signal line indented 4 spaces: backend-specific (★ stars / ↑ points / ✓ accepted / 📄 paper).
- Snippet indented 4 spaces, wrapped to 70 chars, max 3 lines, capped at `max_snippet_chars`.
- Source line indented 4 spaces: `source: github · also: hackernews` (only when `also_in` non-empty).
- Blank line between hits.

**Footer block (always present):**
A horizontal rule (`─────`) followed by these lines, only including lines that have content:
1. `Used:        {comma-list}` — backends that returned hits.
2. `Available:   {comma-list}` — built-in backends currently enabled.
3. `Not enabled: {name (set ENV_VAR), ...}` — backends behind missing env vars (paid tier).
4. `Custom:      {comma-list}` — user-defined backends from config.
5. `Failed:      {name (reason), ...}` — backends that errored.
6. `Skipped:     {name (reason), ...}` — backends rate-limited.
7. `Clamped:     {field old → new, ...}` — agent input that exceeded hard caps.
8. `Truncated:   {N} hits dropped (over budget) — try max_results={X} or max_total_chars={Y}` — hint when format dropped hits.
9. `Hint:        results look thin. Try `backends=["..."]` for ...` — when total hits < 3 AND a stronger backend wasn't used.

**Hard cap enforcement:**
- Final pass: `truncate_safe(&output, req.max_total_chars)`. UTF-8 safe.
- Before that pass, format function tracks running byte count and drops hits from the bottom when adding the next hit would exceed budget.
- **Minimum guarantee:** the first 3 hits always keep their full snippet. After that, snippets shrink to 1 line. After that, snippets disappear (title + URL only). After that, hits disappear from the bottom.
- **Footer is sacrosanct** — always rendered. If output is over budget, drop hits before dropping footer lines.

### 5.3 The `truncate_safe` helper (UTF-8 safe per MEMORY.md resilience rules)

```rust
/// Truncate a string to at most `max_bytes` bytes, on a UTF-8 char boundary.
/// Per MEMORY.md UTF-8 safety rule, NEVER use &s[..n] on user-derived strings.
pub fn truncate_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes { return s.to_string(); }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    s[..end].to_string()
}
```

Used by every snippet builder and the final format pass. Tested with multi-byte input (Vietnamese `ẹ`, Chinese, emoji) before any backend code goes in.

### 5.4 Hint logic (deterministic, no LLM call)

```rust
fn maybe_emit_hint(out: &DispatcherOutput) -> Option<String> {
    // Thin results trigger
    if out.hits.len() >= 3 { return None; }
    // Find a stronger backend that wasn't used
    let stronger: Vec<&str> = out.catalog.available.iter()
        .filter(|id| !out.backends_succeeded.contains(id))
        .map(|id| id.as_str())
        .collect();
    if stronger.is_empty() {
        // Suggest paid if available
        let paid: Vec<&str> = out.catalog.disabled_with_hint.iter()
            .map(|(id, _)| id.as_str()).collect();
        if !paid.is_empty() {
            return Some(format!("results look thin. Set one of: {} for premium search.", paid.join(", ")));
        }
        return None;
    }
    Some(format!(
        "results look thin. Try `backends=[\"{}\"]` for broader coverage.",
        stronger[0]
    ))
}
```

**No LLM call. Pure rule.** Cheap, predictable, easy to test.

### 5.5 Why this format

- **Scannable indices** let the LLM reference results back later: "look at result 2 from the previous search."
- **Source labeling** lets the LLM choose follow-up actions intelligently: a Wikipedia hit is fact, an HN hit is opinion, a GitHub hit is code.
- **Structured signals** (★/↑/✓/📄) are 1-2 chars but communicate trust quickly.
- **Footer = discoverability without an extra round-trip.** The agent always knows what backends exist and how to retry.
- **Truncation transparency** means the agent can dial up `max_results` / `max_total_chars` instead of guessing whether it's seeing everything.
- **No JSON output.** The agent's LLM is better at scanning markdown-like text than parsing JSON for downstream reasoning. JSON is the wrong shape for an LLM consumer.

---

## 6. Governor (`web_search/governor.rs`)

```rust
pub struct Governor {
    last_call: tokio::sync::Mutex<HashMap<BackendId, std::time::Instant>>,
    intervals: HashMap<BackendId, std::time::Duration>,
}

impl Governor {
    pub fn new(intervals: HashMap<BackendId, std::time::Duration>) -> Self { ... }

    pub async fn try_acquire(&self, id: BackendId) -> Result<(), u64> {
        let mut guard = self.last_call.lock().await;
        let now = std::time::Instant::now();
        if let Some(interval) = self.intervals.get(&id) {
            if let Some(last) = guard.get(&id) {
                let elapsed = now.duration_since(*last);
                if elapsed < *interval {
                    return Err((*interval - elapsed).as_millis() as u64);
                }
            }
        }
        guard.insert(id, now);
        Ok(())
    }
}
```

**Defaults (`crates/temm1e-tools/src/web_search/governor.rs` constants):**

```rust
const DEFAULT_INTERVALS: &[(BackendId, u64)] = &[
    (BackendId::HackerNews,    0),    // none
    (BackendId::Wikipedia,     0),    // none
    (BackendId::Github,        6000), // 10/min unauth
    (BackendId::StackOverflow, 300),  // ~3.3/sec
    (BackendId::Reddit,        6000), // 10/min hard
    (BackendId::Marginalia,    1000), // shared anonymous quota
    (BackendId::Arxiv,         3000), // ToS
    (BackendId::Pubmed,        333),  // 3/sec without key
    (BackendId::DuckDuckGo,    6000), // protect Chrome UA
];
```

**Concurrency:** Single mutex on the per-backend last-call map. Hold time is microseconds (one HashMap insert), so contention is irrelevant. No need for sharded locking.

---

## 7. Cache (`web_search/cache.rs`)

```rust
pub struct Cache {
    inner: tokio::sync::RwLock<lru::LruCache<CacheKey, CacheEntry>>,
    ttl: std::time::Duration,
}

type CacheKey = (String, Option<Vec<BackendId>>, TimeRange);

struct CacheEntry {
    hits: Vec<SearchHit>,
    inserted_at: std::time::Instant,
    backends_succeeded: Vec<BackendId>,
}
```

**Capacity:** 256 entries. (Average entry ~5KB → ~1.3 MB cap.)
**TTL:** 5 minutes (config-tunable).
**Lookup:** Read lock + TTL check. If expired, drop and treat as miss.
**Insert:** Write lock, LRU eviction.

The `lru` crate (already used? need to verify) is the right pick. If not in tree, evaluate the `quick_cache` crate as alternative — it's lock-free and slightly faster, but `lru` is simpler.

---

## 8. Tests (`web_search/tests/`)

### 8.1 Unit tests (no network)

**Truncation safety:**
- `truncate_safe_preserves_ascii`
- `truncate_safe_handles_vietnamese_e_at_boundary`
- `truncate_safe_handles_chinese`
- `truncate_safe_handles_emoji`
- `truncate_safe_zero_max_returns_empty`

**Format:**
- `format_renders_empty_results_gracefully`
- `format_renders_full_example_under_budget`
- `format_truncates_to_max_total_chars`
- `format_keeps_first_3_full_at_minimum`
- `format_drops_hits_from_bottom_when_over_budget`
- `format_footer_always_present_even_at_cap`
- `format_footer_lists_used_backends`
- `format_footer_lists_disabled_with_env_hint`
- `format_footer_lists_custom_backends`
- `format_footer_truncation_report_when_dropping`
- `format_emits_hint_when_results_thin_and_stronger_available`
- `format_emits_paid_hint_when_no_free_alternatives`
- `format_no_hint_when_results_sufficient`
- `format_per_hit_snippet_capped_at_max_snippet_chars`

**Clamping & resolution:**
- `clamp_with_log_under_min_clamps_up`
- `clamp_with_log_over_max_clamps_down`
- `clamp_with_log_within_range_no_log`
- `resolve_uses_defaults_when_unspecified`
- `resolve_clamps_oversize_max_results`
- `resolve_clamps_oversize_max_total_chars`
- `resolve_clamps_oversize_max_snippet_chars`
- `resolve_rejects_empty_query`
- `resolve_silently_drops_unknown_backend_names`
- `resolve_silently_defaults_invalid_time_range`
- `resolve_records_clamps_in_resolved_input`

**Dedupe & merge:**
- `dedupe_normalizes_urls`
- `dedupe_strips_utm_params`
- `dedupe_strips_fbclid_and_ref`
- `dedupe_lowercases_host_only`
- `dedupe_drops_trailing_slash`
- `dedupe_keeps_highest_confidence_source`
- `dedupe_merges_also_in_field`
- `merge_applies_include_domains_filter`
- `merge_applies_exclude_domains_filter`
- `merge_include_uses_suffix_match`
- `merge_sort_by_relevance_default`
- `merge_sort_by_date_descending`
- `merge_sort_by_score_raw`
- `merge_truncates_to_max_results`

**Governor:**
- `governor_blocks_within_interval`
- `governor_allows_after_interval_elapses`
- `governor_returns_retry_after_ms`
- `governor_no_deadlock_under_concurrency`
- `governor_per_backend_isolated`

**Cache:**
- `cache_returns_hit_within_ttl`
- `cache_treats_expired_as_miss`
- `cache_evicts_at_capacity`
- `cache_returns_independent_clones`
- `cache_key_distinguishes_backend_filter`
- `cache_key_distinguishes_size_params`

**Dispatcher:**
- `dispatcher_partial_success_returns_ok`
- `dispatcher_total_failure_returns_error_with_clamps_visible`
- `dispatcher_filters_by_requested_backends`
- `dispatcher_skips_disabled_backends`
- `dispatcher_caps_per_backend_raw_hits`
- `dispatcher_default_set_falls_back_when_preferred_disabled`
- `dispatcher_catalog_partitions_correctly`

**Custom backends:**
- `custom_backend_url_template_substitutes_query`
- `custom_backend_validates_required_fields_at_load`
- `custom_backend_disabled_when_env_var_missing`
- `custom_backend_governor_minimum_100ms`
- `custom_backend_appears_in_catalog_under_custom`

Per-backend request-shape tests (mock `reqwest::Client` via `wiremock` or `mockito`):
- `hn_request_includes_tags_story`
- `wikipedia_request_includes_user_agent`
- `github_request_uses_token_when_set`
- `stackexchange_request_uses_advanced_endpoint`
- `reddit_request_uses_old_subdomain`
- `marginalia_request_url_encodes_query`
- `arxiv_request_includes_relevance_sort`
- `pubmed_two_step_protocol`

Per-backend response-parsing tests (saved fixture files in `tests/fixtures/web_search/`):
- `hn_parses_real_response_fixture`
- `wikipedia_parses_excerpt_html`
- `github_parses_repo_metadata`
- `stackexchange_parses_accepted_answer_marker`
- `reddit_parses_self_vs_link_post`
- `arxiv_parses_atom_xml`
- `pubmed_parses_esummary_json`

### 8.2 Integration test (1 live call only)

- `live_hn_algolia_returns_results` — gated on `RUN_LIVE_TESTS=1` env. Hits HN Algolia (no rate limit) with a stable query, asserts >0 hits with required fields. Skipped in CI by default.

### 8.3 Self-test (CLI 10-turn protocol)

Following the MEMORY.md "multi-turn CLI self-test protocol":

```bash
# /tmp/temm1e_websearch_test.sh
(
  echo "Turn 1: search the web for 'rust async runtime benchmarks'"
  sleep 15
  echo "Turn 2: now look up Wikipedia for 'Tokio (software)'"
  sleep 15
  echo "Turn 3: find me github repos about 'sqlite vector search'"
  sleep 15
  echo "Turn 4: search hackernews for 'claude code'"
  sleep 15
  echo "Turn 5: find recent arxiv papers on 'LLM tool use'"
  sleep 15
  echo "Turn 6: what was the first search result you returned to me?"
  sleep 15
  echo "Turn 7: search stackoverflow for 'rust lifetime elision'"
  sleep 15
  echo "Turn 8: find me a marginalia result about sqlite"
  sleep 15
  echo "Turn 9: search reddit for 'rust 2026'"
  sleep 15
  echo "Turn 10: summarize what tools you used and what worked"
  sleep 15
  echo "/quit"
) | ./target/release/temm1e chat 2>&1
```

Validation: each turn produces results from the named backend, Turn 6 demonstrates conversation memory, Turn 10 names every backend used, zero panics, zero "tool not found."

---

## 9. Dependencies — verified 2026-04-12

**Already in `crates/temm1e-tools/Cargo.toml`:**
- `reqwest` ✓ — HTTP client
- `tokio` ✓ — async runtime
- `serde`, `serde_json` ✓ — (de)serialization
- `async-trait` ✓ — for the SearchBackend trait
- `tracing` ✓ — logging
- `chrono` ✓ — timestamp parsing for HN, Reddit, SO
- `regex` ✓ — Wikipedia HTML excerpt stripping AND arXiv Atom XML extraction
- `bytes` ✓ — reqwest body handling
- `url` (transitive via reqwest) ✓ — URL parsing for normalization

**NEW dependencies needed: ZERO.**

Decisions made to avoid new deps:
- **arXiv Atom XML** — instead of `quick-xml`, use a small regex extractor for the 5 fields per `<entry>` (`<id>`, `<title>`, `<summary>`, `<published>`, `<author><name>`). The `regex` crate is already in tree. The XML is well-formed and ~30 lines of regex extracts everything. Trade-off: marginally more fragile if arXiv changes the schema, mitigated by per-entry try/skip.
- **LRU cache** — instead of the `lru` crate, write a tiny manual LRU using `HashMap<K, (V, Instant)>` + insertion-order `VecDeque<K>`. ~50 lines. The cache only needs `get`, `put`, and TTL eviction; full LRU semantics not required for our access pattern.
- **HTML stripping** — 5-line regex `<[^>]+>` to strip the `<span class="searchmatch">` markers Wikipedia returns. No `html_escape` dep needed.

**Net:** Cargo.toml is unchanged. Dependency surface stays exactly where it is.

---

## 10. Error handling philosophy

Following `feedback_no_stubs.md` and `feedback_zero_risk_100_conf.md`:

- **No backend ever panics.** Every `unwrap()` is forbidden in backend code; use `?` and convert to `BackendError`.
- **No backend's failure cascades.** A failed backend produces a `BackendOutcome::Failed` and the dispatcher continues with the rest.
- **The tool only returns `is_error: true` if EVERY backend failed AND zero hits.** Otherwise, partial results are success (with the failure list embedded in the status line).
- **Network errors are first-class.** A timeout is logged and reported, never silenced.
- **Rate limits are first-class.** A 429 returns `BackendError::RateLimited` with the retry-after, and the format function shows `skipped: github (rate limited, retry in 4s)` in the status line.

---

## 11. What this document is NOT

- It is **not** a code listing. It's a spec the implementer follows.
- It is **not** locked. If Phase 1 implementation reveals a better shape (e.g., the dispatcher fan-out wants `FuturesUnordered` instead of `join_all` for streaming), update this doc *and* `IMPLEMENTATION_PLAN.md` to match before continuing.
- It is **not** the architecture review. That's `HARMONY_AUDIT.md`.
