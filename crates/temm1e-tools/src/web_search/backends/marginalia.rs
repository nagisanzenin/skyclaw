//! Marginalia public search API.
//!
//! Endpoint: https://api.marginalia.nu/public/search/{query}?count=N
//! Auth: literal "public" key embedded in URL path.

use super::{fetch_bounded, make_client, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use serde::Deserialize;

pub struct MarginaliaBackend {
    client: reqwest::Client,
}

impl MarginaliaBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
        }
    }
}

impl Default for MarginaliaBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    results: Vec<MarginaliaResult>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MarginaliaResult {
    url: String,
    title: String,
    description: Option<String>,
    #[serde(default)]
    quality: f32,
}

#[async_trait]
impl SearchBackend for MarginaliaBackend {
    fn id(&self) -> BackendId {
        BackendId::Marginalia
    }
    fn name(&self) -> &str {
        "marginalia"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        0.7
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let count = req.per_backend_raw_cap().min(20);
        // Marginalia API uses "public" as the literal key in the path
        let encoded = urlencoding_simple(&req.query);
        let endpoint = format!("https://api.marginalia.nu/public/search/{encoded}?count={count}");
        let request = self.client.get(&endpoint);
        let body = fetch_bounded(&self.client, request).await?;

        let parsed: ApiResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Parse(format!("marginalia json: {e}")))?;

        let hits: Vec<SearchHit> = parsed
            .results
            .into_iter()
            .map(|r| {
                let snippet = r
                    .description
                    .as_deref()
                    .map(|d| truncate_safe(d, 200))
                    .unwrap_or_default();
                SearchHit {
                    title: r.title,
                    url: r.url,
                    snippet,
                    source: BackendId::Marginalia,
                    source_name: "marginalia".into(),
                    published: None,
                    score: r.quality.clamp(0.0, 1.0),
                    signal: Some(HitSignal::MarginaliaQuality { quality: r.quality }),
                    also_in: vec![],
                }
            })
            .collect();
        Ok(hits)
    }
}

/// Minimal URL encoding sufficient for the query path segment.
/// Encodes characters that have special meaning in URLs.
fn urlencoding_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marginalia_backend_construction() {
        let b = MarginaliaBackend::new();
        assert_eq!(b.name(), "marginalia");
    }

    #[test]
    fn url_encoding_simple_query() {
        assert_eq!(urlencoding_simple("hello world"), "hello%20world");
        assert_eq!(urlencoding_simple("rust+programming"), "rust%2Bprogramming");
        assert_eq!(urlencoding_simple("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn parse_marginalia_response() {
        let json = r#"{
            "query": "test",
            "license": "CC-BY-NC-SA",
            "results": [
                {
                    "url": "https://example.com/blog",
                    "title": "Blog post",
                    "description": "A small-web essay",
                    "quality": 0.8
                }
            ]
        }"#;
        let parsed: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].quality, 0.8);
    }
}
