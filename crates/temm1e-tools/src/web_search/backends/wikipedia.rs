//! Wikipedia REST API v1 search.
//!
//! Endpoint: https://en.wikipedia.org/w/rest.php/v1/search/page
//! No auth, generous rate limit, requires descriptive User-Agent.

use super::{fetch_bounded, make_client, strip_html, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use serde::Deserialize;

pub struct WikipediaBackend {
    client: reqwest::Client,
}

impl WikipediaBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
        }
    }
}

impl Default for WikipediaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    pages: Vec<ApiPage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiPage {
    id: u64,
    key: String,
    title: String,
    excerpt: Option<String>,
    description: Option<String>,
}

#[async_trait]
impl SearchBackend for WikipediaBackend {
    fn id(&self) -> BackendId {
        BackendId::Wikipedia
    }
    fn name(&self) -> &str {
        "wikipedia"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        1.0
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let lang = req.language.as_deref().unwrap_or("en");
        let endpoint = format!("https://{lang}.wikipedia.org/w/rest.php/v1/search/page");
        let limit = req.per_backend_raw_cap().min(50);

        let request = self
            .client
            .get(&endpoint)
            .query(&[("q", req.query.as_str()), ("limit", &limit.to_string())]);

        let body = fetch_bounded(&self.client, request).await?;
        let parsed: ApiResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Parse(format!("wikipedia json: {e}")))?;

        let total = parsed.pages.len();
        let hits: Vec<SearchHit> = parsed
            .pages
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let url = format!("https://{lang}.wikipedia.org/wiki/{}", p.key);
                let excerpt = p.excerpt.as_deref().map(strip_html).unwrap_or_default();
                let description = p.description.clone();
                let snippet = match (&excerpt, &description) {
                    (e, Some(d)) if !e.is_empty() => truncate_safe(&format!("{e} — {d}"), 300),
                    (e, None) if !e.is_empty() => truncate_safe(e, 300),
                    (_, Some(d)) => truncate_safe(d, 300),
                    _ => String::new(),
                };
                let score = if total == 0 {
                    0.0
                } else {
                    1.0 - (i as f32 / total as f32)
                };
                SearchHit {
                    title: p.title,
                    url,
                    snippet,
                    source: BackendId::Wikipedia,
                    source_name: "wikipedia".into(),
                    published: None,
                    score,
                    signal: Some(HitSignal::Wikipedia { description }),
                    also_in: vec![],
                }
            })
            .collect();

        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wikipedia_backend_construction() {
        let b = WikipediaBackend::new();
        assert_eq!(b.name(), "wikipedia");
        assert!(b.enabled());
    }

    #[test]
    fn parse_real_response_fixture() {
        let json = r#"{
            "pages": [
                {
                    "id": 12345,
                    "key": "Tokio_(software)",
                    "title": "Tokio (software)",
                    "excerpt": "An <span class=\"searchmatch\">async</span> runtime for Rust",
                    "description": "Asynchronous runtime for the Rust programming language"
                }
            ]
        }"#;
        let parsed: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.pages.len(), 1);
        assert_eq!(parsed.pages[0].title, "Tokio (software)");
        assert!(parsed.pages[0].excerpt.is_some());
    }

    #[test]
    fn parse_excerpt_html_strips() {
        let s = strip_html("An <span class=\"searchmatch\">async</span> runtime");
        assert_eq!(s, "An async runtime");
    }
}
