//! URL normalization for dedupe — strips tracking params, lowercases host,
//! drops trailing slash, drops fragment.

use reqwest::Url;

const TRACKING_PREFIXES: &[&str] = &["utm_", "fbclid", "gclid", "ref", "ref_src", "_ga"];

/// Normalize a URL string for dedupe key purposes.
/// Returns the original string if parsing fails (so we don't crash on bad URLs).
pub fn normalize(raw: &str) -> String {
    let mut parsed = match Url::parse(raw) {
        Ok(u) => u,
        Err(_) => return raw.trim().to_lowercase(),
    };

    // Drop fragment
    parsed.set_fragment(None);

    // Lowercase host
    if let Some(host) = parsed.host_str() {
        let host_lower = host.to_lowercase();
        let _ = parsed.set_host(Some(&host_lower));
    }

    // Strip tracking query params
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| {
            let kl = k.to_ascii_lowercase();
            !TRACKING_PREFIXES.iter().any(|p| kl.starts_with(p))
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    parsed.set_query(None);
    if !kept.is_empty() {
        let mut q = parsed.query_pairs_mut();
        for (k, v) in &kept {
            q.append_pair(k, v);
        }
    }

    let mut s = parsed.to_string();
    // Drop trailing slash on path (but not on bare host)
    if s.ends_with('/') && parsed.path() != "/" {
        s.pop();
    }
    s
}

/// Check if a host matches any of the suffix patterns.
/// E.g., "github.com" matches "docs.github.com" via suffix.
pub fn host_matches_suffix(host: &str, patterns: &[String]) -> bool {
    let host_l = host.to_ascii_lowercase();
    patterns.iter().any(|p| {
        let pl = p.to_ascii_lowercase();
        host_l == pl || host_l.ends_with(&format!(".{pl}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_utm() {
        let n = normalize("https://example.com/page?utm_source=foo&utm_medium=bar&id=42");
        assert!(!n.contains("utm_"));
        assert!(n.contains("id=42"));
    }

    #[test]
    fn normalize_strips_fbclid() {
        let n = normalize("https://example.com/page?fbclid=abc123");
        assert!(!n.contains("fbclid"));
    }

    #[test]
    fn normalize_strips_ref() {
        let n = normalize("https://example.com/page?ref=foo");
        assert!(!n.contains("ref="));
    }

    #[test]
    fn normalize_lowercases_host() {
        let n = normalize("https://EXAMPLE.COM/Path");
        assert!(n.starts_with("https://example.com/"));
    }

    #[test]
    fn normalize_drops_trailing_slash() {
        let n = normalize("https://example.com/page/");
        assert!(!n.ends_with('/'));
        let bare = normalize("https://example.com/");
        assert!(bare.ends_with('/'));
    }

    #[test]
    fn normalize_drops_fragment() {
        let n = normalize("https://example.com/page#section");
        assert!(!n.contains('#'));
    }

    #[test]
    fn normalize_returns_lowercase_on_parse_failure() {
        let n = normalize("not a url");
        assert_eq!(n, "not a url");
    }

    #[test]
    fn host_matches_exact() {
        assert!(host_matches_suffix("github.com", &["github.com".into()]));
    }

    #[test]
    fn host_matches_subdomain() {
        assert!(host_matches_suffix(
            "docs.github.com",
            &["github.com".into()]
        ));
    }

    #[test]
    fn host_does_not_match_unrelated() {
        assert!(!host_matches_suffix(
            "notgithub.com",
            &["github.com".into()]
        ));
        assert!(!host_matches_suffix("example.org", &["github.com".into()]));
    }

    #[test]
    fn host_case_insensitive() {
        assert!(host_matches_suffix("GitHub.COM", &["github.com".into()]));
    }
}
