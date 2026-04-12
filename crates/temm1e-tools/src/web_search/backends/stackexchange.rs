//! Stack Exchange API 2.3 search.
//!
//! Endpoint: https://api.stackexchange.com/2.3/search/advanced
//! No auth up to 300 req/day per IP.

use super::{fetch_bounded, make_client, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use serde::Deserialize;

pub struct StackOverflowBackend {
    client: reqwest::Client,
}

impl StackOverflowBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
        }
    }
}

impl Default for StackOverflowBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    items: Vec<Question>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Question {
    title: String,
    link: String,
    score: i32,
    answer_count: u32,
    is_answered: bool,
    #[serde(default)]
    accepted_answer_id: Option<u64>,
    #[serde(default)]
    tags: Vec<String>,
    last_activity_date: Option<i64>,
}

#[async_trait]
impl SearchBackend for StackOverflowBackend {
    fn id(&self) -> BackendId {
        BackendId::StackOverflow
    }
    fn name(&self) -> &str {
        "stackoverflow"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        1.0
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let endpoint = "https://api.stackexchange.com/2.3/search/advanced";
        let pagesize = req.per_backend_raw_cap().min(50);

        let request = self.client.get(endpoint).query(&[
            ("q", req.query.as_str()),
            ("site", "stackoverflow"),
            ("pagesize", &pagesize.to_string()),
            ("order", "desc"),
            ("sort", "relevance"),
        ]);

        let body = fetch_bounded(&self.client, request).await?;
        let parsed: ApiResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Parse(format!("stackexchange json: {e}")))?;

        let hits: Vec<SearchHit> = parsed
            .items
            .into_iter()
            .map(|q| {
                let accepted = q.accepted_answer_id.is_some();
                let tags_str = if q.tags.is_empty() {
                    String::new()
                } else {
                    format!(" · tags: {}", q.tags.join(", "))
                };
                let acc_marker = if accepted { " (accepted ✓)" } else { "" };
                let snippet = truncate_safe(
                    &format!(
                        "{} score · {} answers{}{}",
                        q.score, q.answer_count, tags_str, acc_marker
                    ),
                    200,
                );
                let published = q.last_activity_date.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| dt.format("%Y-%m-%d").to_string())
                });
                let score = (q.score as f32 / 200.0).clamp(0.0, 1.0);
                SearchHit {
                    title: html_unescape(&q.title),
                    url: q.link,
                    snippet,
                    source: BackendId::StackOverflow,
                    source_name: "stackoverflow".into(),
                    published,
                    score,
                    signal: Some(HitSignal::StackOverflowScore {
                        score: q.score,
                        answers: q.answer_count,
                        accepted,
                    }),
                    also_in: vec![],
                }
            })
            .collect();

        Ok(hits)
    }
}

/// Decode common HTML entities found in StackExchange titles.
fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_backend_construction() {
        let b = StackOverflowBackend::new();
        assert_eq!(b.name(), "stackoverflow");
    }

    #[test]
    fn parse_accepted_answer_marker() {
        let json = r#"{
            "items": [
                {
                    "title": "How do I parse JSON in Rust?",
                    "link": "https://stackoverflow.com/q/12345",
                    "score": 42,
                    "answer_count": 5,
                    "is_answered": true,
                    "accepted_answer_id": 67890,
                    "tags": ["rust", "json", "serde"],
                    "last_activity_date": 1735689600
                }
            ]
        }"#;
        let parsed: ApiResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.items[0].accepted_answer_id.is_some());
        assert_eq!(parsed.items[0].score, 42);
    }

    #[test]
    fn html_unescape_basic() {
        assert_eq!(
            html_unescape("Why does &lt;Vec&gt; work like that?"),
            "Why does <Vec> work like that?"
        );
        assert_eq!(html_unescape("Tom &amp; Jerry"), "Tom & Jerry");
    }
}
