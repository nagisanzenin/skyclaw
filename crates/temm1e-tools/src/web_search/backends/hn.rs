//! HackerNews via Algolia search API.
//!
//! Endpoint: https://hn.algolia.com/api/v1/search
//! No auth, no rate limit (Algolia infrastructure).

use super::{fetch_bounded, make_client, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use serde::Deserialize;

pub struct HackerNewsBackend {
    client: reqwest::Client,
}

impl HackerNewsBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
        }
    }
}

impl Default for HackerNewsBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    hits: Vec<ApiHit>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiHit {
    #[serde(rename = "objectID")]
    object_id: String,
    title: Option<String>,
    url: Option<String>,
    story_url: Option<String>,
    points: Option<u32>,
    num_comments: Option<u32>,
    #[serde(default)]
    author: Option<String>,
    created_at_i: Option<i64>,
    story_text: Option<String>,
}

#[async_trait]
impl SearchBackend for HackerNewsBackend {
    fn id(&self) -> BackendId {
        BackendId::HackerNews
    }
    fn name(&self) -> &str {
        "hackernews"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        1.0
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let endpoint = match req.time_range {
            TimeRange::Day | TimeRange::Week => "https://hn.algolia.com/api/v1/search_by_date",
            _ => "https://hn.algolia.com/api/v1/search",
        };

        let per_page = req.per_backend_raw_cap().min(50);
        let mut params: Vec<(&str, String)> = vec![
            ("query", req.query.clone()),
            ("tags", "story".to_string()),
            ("hitsPerPage", per_page.to_string()),
        ];

        if let Some(cutoff) = req.time_range.cutoff_secs() {
            let now = chrono::Utc::now().timestamp();
            params.push(("numericFilters", format!("created_at_i>{}", now - cutoff)));
        }

        let request = self.client.get(endpoint).query(&params);
        let body = fetch_bounded(&self.client, request).await?;
        let parsed: ApiResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Parse(format!("hn json: {e}")))?;

        let hits: Vec<SearchHit> = parsed
            .hits
            .into_iter()
            .filter_map(|h| {
                let title = h.title?;
                let url = h.url.or(h.story_url).unwrap_or_else(|| {
                    format!("https://news.ycombinator.com/item?id={}", h.object_id)
                });
                let points = h.points.unwrap_or(0);
                let comments = h.num_comments.unwrap_or(0);
                let snippet = h
                    .story_text
                    .as_deref()
                    .map(|s| truncate_safe(&strip_basic_html(s), 200))
                    .unwrap_or_default();
                let published = h.created_at_i.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| dt.format("%Y-%m-%d").to_string())
                });
                let score = (points as f32 / 500.0).clamp(0.0, 1.0);
                Some(SearchHit {
                    title,
                    url,
                    snippet,
                    source: BackendId::HackerNews,
                    source_name: "hackernews".into(),
                    published,
                    score,
                    signal: Some(HitSignal::HnPoints { points, comments }),
                    also_in: vec![],
                })
            })
            .collect();

        Ok(hits)
    }
}

fn strip_basic_html(s: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"<[^>]+>").expect("static regex"));
    re.replace_all(s, "").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hn_backend_construction() {
        let b = HackerNewsBackend::new();
        assert_eq!(b.name(), "hackernews");
        assert!(b.enabled());
        assert_eq!(b.default_weight(), 1.0);
        assert!(matches!(b.id(), BackendId::HackerNews));
    }

    #[test]
    fn parse_real_response_fixture() {
        let json = r#"{
            "hits": [
                {
                    "objectID": "12345",
                    "title": "Test Story",
                    "url": "https://example.com/story",
                    "points": 250,
                    "num_comments": 42,
                    "author": "testuser",
                    "created_at_i": 1735689600,
                    "story_text": null
                }
            ]
        }"#;
        let parsed: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.hits.len(), 1);
        assert_eq!(parsed.hits[0].title.as_deref(), Some("Test Story"));
        assert_eq!(parsed.hits[0].points, Some(250));
    }

    #[test]
    fn parse_handles_null_url_show_hn() {
        let json = r#"{
            "hits": [
                {
                    "objectID": "99999",
                    "title": "Show HN: My project",
                    "url": null,
                    "points": 10,
                    "num_comments": 5,
                    "author": "showhn",
                    "created_at_i": 1735689600,
                    "story_text": "<p>Hello world</p>"
                }
            ]
        }"#;
        let parsed: ApiResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.hits[0].url.is_none());
    }

    #[test]
    fn strip_basic_html_works() {
        let s = strip_basic_html("<p>hello <b>world</b></p>");
        assert_eq!(s, "hello world");
    }
}
