//! GitHub Search API.
//!
//! Endpoint: https://api.github.com/search/repositories
//! No auth (10/min) or with GITHUB_TOKEN (30/min).
//! Required: User-Agent header.

use super::{default_user_agent, fetch_bounded, make_client, truncate_safe, SearchBackend};
use crate::web_search::types::*;
use async_trait::async_trait;
use serde::Deserialize;

pub struct GithubBackend {
    client: reqwest::Client,
    token: Option<String>,
}

impl GithubBackend {
    pub fn new() -> Self {
        Self {
            client: make_client(),
            token: std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty()),
        }
    }
}

impl Default for GithubBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    items: Vec<Repo>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Repo {
    full_name: String,
    html_url: String,
    description: Option<String>,
    stargazers_count: u64,
    language: Option<String>,
    pushed_at: Option<String>,
}

#[async_trait]
impl SearchBackend for GithubBackend {
    fn id(&self) -> BackendId {
        BackendId::Github
    }
    fn name(&self) -> &str {
        "github"
    }
    fn enabled(&self) -> bool {
        true
    }
    fn default_weight(&self) -> f32 {
        1.0
    }

    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError> {
        let endpoint = "https://api.github.com/search/repositories";
        let per_page = req.per_backend_raw_cap().min(50);

        let mut request = self
            .client
            .get(endpoint)
            .header("User-Agent", default_user_agent())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(&[
                ("q", req.query.as_str()),
                ("per_page", &per_page.to_string()),
                ("sort", "stars"),
                ("order", "desc"),
            ]);

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let body = fetch_bounded(&self.client, request).await?;
        let parsed: ApiResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Parse(format!("github json: {e}")))?;

        let hits: Vec<SearchHit> = parsed
            .items
            .into_iter()
            .map(|r| {
                let stars = r.stargazers_count.min(u32::MAX as u64) as u32;
                let snippet = r
                    .description
                    .as_deref()
                    .map(|d| truncate_safe(d, 200))
                    .unwrap_or_default();
                // log10(stars+1) / 6 → 1M stars ≈ 1.0
                let score = ((stars as f32 + 1.0).log10() / 6.0).clamp(0.0, 1.0);
                SearchHit {
                    title: r.full_name,
                    url: r.html_url,
                    snippet,
                    source: BackendId::Github,
                    source_name: "github".into(),
                    published: r
                        .pushed_at
                        .map(|p| p.split('T').next().unwrap_or(&p).to_string()),
                    score,
                    signal: Some(HitSignal::GithubStars {
                        stars,
                        language: r.language,
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
    fn github_backend_construction() {
        let b = GithubBackend::new();
        assert_eq!(b.name(), "github");
        assert!(b.enabled());
    }

    #[test]
    fn parse_repo_metadata() {
        let json = r#"{
            "items": [
                {
                    "full_name": "tokio-rs/tokio",
                    "html_url": "https://github.com/tokio-rs/tokio",
                    "description": "An async runtime for Rust",
                    "stargazers_count": 25000,
                    "language": "Rust",
                    "pushed_at": "2026-04-10T12:34:56Z"
                }
            ]
        }"#;
        let parsed: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].full_name, "tokio-rs/tokio");
        assert_eq!(parsed.items[0].stargazers_count, 25000);
    }

    #[test]
    fn parse_handles_null_description() {
        let json = r#"{
            "items": [
                {
                    "full_name": "x/y",
                    "html_url": "https://github.com/x/y",
                    "description": null,
                    "stargazers_count": 0,
                    "language": null,
                    "pushed_at": null
                }
            ]
        }"#;
        let parsed: ApiResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.items[0].description.is_none());
    }
}
