//! Reddit JSON search.
//!
//! Endpoint: https://old.reddit.com/search.json
//! No auth, 10/min unauth limit. Custom User-Agent required.

use super::{default_user_agent, fetch_bounded, make_client, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use serde::Deserialize;

pub struct RedditBackend {
    client: reqwest::Client,
}

impl RedditBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
        }
    }
}

impl Default for RedditBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ListingResponse {
    data: ListingData,
}

#[derive(Debug, Deserialize)]
struct ListingData {
    children: Vec<Child>,
}

#[derive(Debug, Deserialize)]
struct Child {
    data: Post,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Post {
    title: String,
    selftext: Option<String>,
    url: String,
    permalink: String,
    score: i32,
    num_comments: u32,
    subreddit: String,
    created_utc: Option<f64>,
    is_self: Option<bool>,
}

#[async_trait]
impl SearchBackend for RedditBackend {
    fn id(&self) -> BackendId {
        BackendId::Reddit
    }
    fn name(&self) -> &str {
        "reddit"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        0.85
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let endpoint = "https://old.reddit.com/search.json";
        let limit = req.per_backend_raw_cap().min(50);

        // Reddit time range: hour, day, week, month, year, all
        let t_param = match req.time_range {
            TimeRange::Day => "day",
            TimeRange::Week => "week",
            TimeRange::Month => "month",
            TimeRange::Year => "year",
            TimeRange::All => "all",
        };

        let request = self
            .client
            .get(endpoint)
            .header("User-Agent", default_user_agent())
            .query(&[
                ("q", req.query.as_str()),
                ("limit", &limit.to_string()),
                ("raw_json", "1"),
                ("sort", "relevance"),
                ("t", t_param),
            ]);

        let body = fetch_bounded(&self.client, request).await?;
        let parsed: ListingResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Parse(format!("reddit json: {e}")))?;

        let hits: Vec<SearchHit> = parsed
            .data
            .children
            .into_iter()
            .map(|c| {
                let p = c.data;
                let is_self = p.is_self.unwrap_or(false);
                let url = if is_self {
                    format!("https://reddit.com{}", p.permalink)
                } else {
                    p.url
                };
                let snippet = if !p.selftext.as_deref().unwrap_or("").is_empty() {
                    truncate_safe(p.selftext.as_deref().unwrap_or(""), 200)
                } else {
                    format!("Link post in r/{}", p.subreddit)
                };
                let published = p.created_utc.and_then(|ts| {
                    chrono::DateTime::from_timestamp(ts as i64, 0)
                        .map(|dt| dt.format("%Y-%m-%d").to_string())
                });
                let score = (p.score as f32 / 1000.0).clamp(0.0, 1.0);
                SearchHit {
                    title: p.title,
                    url,
                    snippet,
                    source: BackendId::Reddit,
                    source_name: "reddit".into(),
                    published,
                    score,
                    signal: Some(HitSignal::RedditUpvotes {
                        ups: p.score,
                        comments: p.num_comments,
                        subreddit: p.subreddit,
                    }),
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
    fn reddit_backend_construction() {
        let b = RedditBackend::new();
        assert_eq!(b.name(), "reddit");
    }

    #[test]
    fn parse_self_vs_link_post() {
        let json = r#"{
            "data": {
                "children": [
                    {"data": {
                        "title": "Self post",
                        "selftext": "Body text here",
                        "url": "https://reddit.com/r/rust/comments/abc",
                        "permalink": "/r/rust/comments/abc/self_post/",
                        "score": 100,
                        "num_comments": 25,
                        "subreddit": "rust",
                        "created_utc": 1735689600.0,
                        "is_self": true
                    }},
                    {"data": {
                        "title": "Link post",
                        "selftext": "",
                        "url": "https://example.com/article",
                        "permalink": "/r/rust/comments/def/link_post/",
                        "score": 50,
                        "num_comments": 10,
                        "subreddit": "rust",
                        "created_utc": 1735689600.0,
                        "is_self": false
                    }}
                ]
            }
        }"#;
        let parsed: ListingResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.data.children.len(), 2);
        assert_eq!(parsed.data.children[0].data.is_self, Some(true));
        assert_eq!(parsed.data.children[1].data.is_self, Some(false));
    }
}
