//! SearchBackend trait + per-backend implementations.

use crate::web_search::types::*;
use async_trait::async_trait;

pub mod arxiv;
pub mod github;
pub mod hn;
pub mod marginalia;
pub mod pubmed;
pub mod reddit;
pub mod stackexchange;
pub mod wikipedia;

pub use arxiv::ArxivBackend;
pub use github::GithubBackend;
pub use hn::HackerNewsBackend;
pub use marginalia::MarginaliaBackend;
pub use pubmed::PubmedBackend;
pub use reddit::RedditBackend;
pub use stackexchange::StackOverflowBackend;
pub use wikipedia::WikipediaBackend;

/// A search backend that can answer a SearchRequest.
#[async_trait]
pub trait SearchBackend: Send + Sync {
    /// Stable backend identifier (enum variant).
    fn id(&self) -> BackendId;

    /// Display name as shown to the agent (matches `BackendId::from_str`).
    /// For custom backends, the user's chosen id.
    fn name(&self) -> &str;

    /// Whether this backend is currently usable.
    /// Built-in free backends always return true.
    /// Paid backends return false when their env var is not set.
    /// Custom backends return false when their referenced env vars are missing.
    fn enabled(&self) -> bool;

    /// Default merge weight in 0.0..=1.0. Higher = more trusted.
    fn default_weight(&self) -> f32;

    /// True if this is a user-defined custom backend (for footer categorization).
    fn is_custom(&self) -> bool {
        false
    }

    /// If disabled, the env var name to suggest in the footer ("set EXA_API_KEY").
    fn disabled_env_hint(&self) -> Option<&str> {
        None
    }

    /// Run a search. The dispatcher wraps this in a timeout.
    async fn search(&self, req: &SearchRequest) -> Result<Vec<SearchHit>, BackendError>;
}

/// Build a fresh `reqwest::Client` configured for backend use.
pub(crate) fn make_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            crate::web_search::types::DEFAULT_BACKEND_TIMEOUT_SECS,
        ))
        .user_agent(default_user_agent())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub(crate) fn default_user_agent() -> String {
    format!(
        "Tem/{} (https://github.com/temm1e-labs/temm1e)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Bounded-body fetch helper used by all HTTP backends.
/// Caps response read at MAX_BACKEND_RESPONSE_BYTES to prevent memory blowout.
pub(crate) async fn fetch_bounded(
    client: &reqwest::Client,
    request: reqwest::RequestBuilder,
) -> Result<String, BackendError> {
    let _ = client; // future use; pass through for symmetry
    let response = request
        .send()
        .await
        .map_err(|e| BackendError::Network(e.to_string()))?;
    let status = response.status();
    if status.as_u16() == 429 {
        return Err(BackendError::RateLimited {
            retry_after_ms: 60_000,
        });
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(BackendError::Http {
            status: status.as_u16(),
            body,
        });
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| BackendError::Network(e.to_string()))?;
    let capped = if bytes.len() > MAX_BACKEND_RESPONSE_BYTES {
        &bytes[..MAX_BACKEND_RESPONSE_BYTES]
    } else {
        &bytes[..]
    };
    String::from_utf8(capped.to_vec()).map_err(|e| BackendError::Parse(e.to_string()))
}

/// UTF-8 safe truncation for snippet building (mirrors format::truncate_safe).
pub(crate) fn truncate_safe(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Strip simple HTML tags using a regex (Wikipedia returns <span> markers).
pub(crate) fn strip_html(s: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"<[^>]+>").expect("static regex"));
    re.replace_all(s, "").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_safe_in_backends_module() {
        assert_eq!(truncate_safe("hello world", 5), "hello");
        assert_eq!(truncate_safe("abc", 100), "abc");
    }

    #[test]
    fn strip_html_removes_span() {
        let s = strip_html("hello <span class=\"x\">world</span>");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn strip_html_removes_nested() {
        let s = strip_html("<b>bold</b> and <i>italic</i>");
        assert_eq!(s, "bold and italic");
    }

    #[test]
    fn strip_html_handles_no_tags() {
        let s = strip_html("just plain text");
        assert_eq!(s, "just plain text");
    }

    #[test]
    fn user_agent_contains_version() {
        let ua = default_user_agent();
        assert!(ua.starts_with("Tem/"));
        assert!(ua.contains("temm1e-labs"));
    }
}
