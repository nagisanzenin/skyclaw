//! `web_search` tool — multi-backend web search with zero-key defaults.
//!
//! See `docs/web_search/IMPLEMENTATION_PLAN.md` and friends for the design.
//!
//! The agent sees ONE tool, `web_search`, with optional params for backend
//! selection, size knobs, and per-call modifiers. Underneath, a dispatcher
//! fans out across N backends in parallel, merges/dedupes results, and
//! formats LLM-optimized output with discoverability footer.

pub mod backends;
pub mod cache;
pub mod dispatcher;
pub mod format;
pub mod governor;
pub mod types;
pub mod url_norm;

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

use crate::web_search::backends::SearchBackend;
use crate::web_search::cache::Cache;
use crate::web_search::dispatcher::{Dispatcher, DispatcherConfig};
use crate::web_search::governor::{default_intervals, Governor};
use crate::web_search::types::*;

use temm1e_core::types::error::Temm1eError;
use temm1e_core::{Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

const TOOL_NAME: &str = "web_search";

const TOOL_DESCRIPTION: &str = "Search the web. Returns a ranked list of results merged from multiple specialized sources. \
Works out of the box with no API keys or setup. Use this when you need current information, documentation, code, \
discussions, research papers, or facts that aren't in your training data. The output footer always tells you which \
sources are available, which were used, and how to retry with different parameters if results look thin.\n\n\
Sources (auto-picked from a sensible default mix; override via `backends`):\n\
  - hackernews:    tech news, opinions, Show HN, Ask HN\n\
  - wikipedia:     facts, definitions, entities, history, biography\n\
  - github:        code, repositories, issues, projects\n\
  - stackoverflow: programming Q&A, error messages, how-tos\n\
  - reddit:        community discussions, opinions, niche subjects\n\
  - marginalia:    blogs, essays, small-web, long-form writing\n\
  - arxiv:         academic papers (CS, math, physics)\n\
  - pubmed:        biomedical and life sciences research\n\n\
When auto results look thin, retry with explicit `backends=[...]`. The footer will hint which alternatives exist. \
Result size is bounded by three knobs: `max_results` (1-30, default 10), `max_total_chars` (1000-16000, default 8000), \
and `max_snippet_chars` (50-500, default 200).";

pub struct WebSearchTool {
    dispatcher: Arc<Dispatcher>,
}

impl WebSearchTool {
    /// Construct with the default Phase 1 backend set + default config.
    pub fn new() -> Self {
        Self::with_config(WebSearchToolConfig::default())
    }

    pub fn with_config(cfg: WebSearchToolConfig) -> Self {
        let backends: Vec<Arc<dyn SearchBackend>> = vec![
            Arc::new(backends::HackerNewsBackend::new()),
            Arc::new(backends::WikipediaBackend::new()),
            Arc::new(backends::GithubBackend::new()),
            Arc::new(backends::StackOverflowBackend::new()),
            Arc::new(backends::RedditBackend::new()),
            Arc::new(backends::MarginaliaBackend::new()),
            Arc::new(backends::ArxivBackend::new()),
            Arc::new(backends::PubmedBackend::new()),
        ];

        let governor = Arc::new(Governor::new(default_intervals()));
        let cache = Arc::new(Cache::new(
            DEFAULT_CACHE_CAPACITY,
            Duration::from_secs(cfg.cache_ttl_secs),
        ));

        let mut disp_config = DispatcherConfig::default();
        if let Some(v) = cfg.default_max_results {
            disp_config.default_max_results = v.clamp(MIN_MAX_RESULTS, HARD_MAX_RESULTS);
        }
        if let Some(v) = cfg.default_max_total_chars {
            disp_config.default_max_total_chars =
                v.clamp(MIN_MAX_TOTAL_CHARS, HARD_MAX_TOTAL_CHARS);
        }
        if let Some(v) = cfg.default_max_snippet_chars {
            disp_config.default_max_snippet_chars =
                v.clamp(MIN_MAX_SNIPPET_CHARS, HARD_MAX_SNIPPET_CHARS);
        }
        disp_config.backend_timeout = Duration::from_secs(cfg.backend_timeout_secs);
        if !cfg.default_backends.is_empty() {
            disp_config.default_backends = cfg.default_backends;
        }

        let dispatcher = Dispatcher::new(backends, governor, cache, disp_config);
        Self {
            dispatcher: Arc::new(dispatcher),
        }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Construction-time config for WebSearchTool.
/// Lifted from `[tools.web_search]` in `temm1e.toml` if present.
#[derive(Debug, Clone)]
pub struct WebSearchToolConfig {
    pub default_max_results: Option<usize>,
    pub default_max_total_chars: Option<usize>,
    pub default_max_snippet_chars: Option<usize>,
    pub backend_timeout_secs: u64,
    pub cache_ttl_secs: u64,
    pub default_backends: Vec<String>,
}

impl Default for WebSearchToolConfig {
    fn default() -> Self {
        Self {
            default_max_results: None,
            default_max_total_chars: None,
            default_max_snippet_chars: None,
            backend_timeout_secs: DEFAULT_BACKEND_TIMEOUT_SECS,
            cache_ttl_secs: DEFAULT_CACHE_TTL_SECS,
            default_backends: vec![],
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        TOOL_DESCRIPTION
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
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
                    "description": "Optional. Specific backends to query, e.g. ['hackernews','github'] for a tech query or ['wikipedia'] for a fact. Unknown names are silently ignored. Omit for the default mix."
                },
                "time_range": {
                    "type": "string",
                    "enum": ["day","week","month","year","all"],
                    "description": "Optional. Restrict to recent results. Default: all."
                },
                "category": {
                    "type": "string",
                    "enum": ["company","research_paper","news","personal_site","financial_report","people","code"],
                    "description": "Optional. Category hint. Backends that don't support it ignore silently."
                },
                "language": {
                    "type": "string",
                    "description": "Optional. ISO 639-1 language code, e.g. 'en', 'vi', 'ja'."
                },
                "region": {
                    "type": "string",
                    "description": "Optional. ISO 3166-1 country code, e.g. 'us', 'vn', 'jp'."
                },
                "include_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional. Only return results from these domains (suffix match)."
                },
                "exclude_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional. Drop results from these domains (suffix match)."
                },
                "sort": {
                    "type": "string",
                    "enum": ["relevance","date","score"],
                    "description": "Optional. Sort order. Default: relevance."
                }
            },
            "required": ["query"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![],
            network_access: vec![
                "hn.algolia.com".into(),
                "en.wikipedia.org".into(),
                "api.github.com".into(),
                "api.stackexchange.com".into(),
                "old.reddit.com".into(),
                "www.reddit.com".into(),
                "api.marginalia.nu".into(),
                "export.arxiv.org".into(),
                "eutils.ncbi.nlm.nih.gov".into(),
            ],
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let raw: RawSearchInput = serde_json::from_value(input.arguments)
            .map_err(|e| Temm1eError::Tool(format!("web_search: invalid arguments: {e}")))?;

        tracing::info!(query = %raw.query, "web_search dispatch");
        let output = self.dispatcher.search(raw).await;
        let content = format::render(&output);
        let is_error = output.input_error.is_some()
            || (output.hits.is_empty() && output.backends_succeeded.is_empty());

        Ok(ToolOutput { content, is_error })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_search_tool_construction() {
        let t = WebSearchTool::new();
        assert_eq!(t.name(), "web_search");
        assert!(!t.description().is_empty());
    }

    #[test]
    fn schema_includes_all_params() {
        let t = WebSearchTool::new();
        let schema = t.parameters_schema();
        let props = schema.get("properties").unwrap();
        for field in [
            "query",
            "max_results",
            "max_total_chars",
            "max_snippet_chars",
            "backends",
            "time_range",
            "category",
            "language",
            "region",
            "include_domains",
            "exclude_domains",
            "sort",
        ] {
            assert!(props.get(field).is_some(), "schema missing field: {field}");
        }
    }

    #[test]
    fn schema_required_is_only_query() {
        let t = WebSearchTool::new();
        let schema = t.parameters_schema();
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str(), Some("query"));
    }

    #[test]
    fn declarations_lists_phase_1_domains() {
        let t = WebSearchTool::new();
        let decl = t.declarations();
        assert!(decl.network_access.contains(&"hn.algolia.com".to_string()));
        assert!(decl
            .network_access
            .contains(&"en.wikipedia.org".to_string()));
        assert!(!decl.shell_access);
    }
}
