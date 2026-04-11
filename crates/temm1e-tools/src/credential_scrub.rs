//! Credential scrubber — removes sensitive data before it reaches the LLM.
//!
//! Applied to all browser observations that follow credential injection so that
//! passwords, tokens, API keys, and auth headers never leak into the agent's
//! conversation context.

use std::sync::LazyLock;

use regex::Regex;

/// Matches sensitive URL query parameters (token, key, secret, password, etc.).
static SENSITIVE_URL_PARAMS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(token|key|secret|password|passwd|pwd|auth|access_token|api_key|session_id|csrf|nonce)=([^&\s]+)",
    )
    .expect("invalid SENSITIVE_URL_PARAMS regex")
});

/// Matches Authorization and similar auth headers.
static AUTH_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(authorization|x-api-key|x-auth-token):[^\n]+")
        .expect("invalid AUTH_HEADER regex")
});

/// Matches common API key patterns across major providers.
static API_KEY_PATTERNS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)(",
        r"sk-ant-[a-zA-Z0-9_-]{20,}", // Anthropic
        r"|sk-or-[a-zA-Z0-9_-]{20,}", // OpenRouter
        r"|sk-[a-zA-Z0-9_-]{20,}",    // OpenAI / generic
        r"|key-[a-zA-Z0-9_-]{20,}",   // Generic key-
        r"|ghp_[a-zA-Z0-9]{36}",      // GitHub PAT
        r"|gho_[a-zA-Z0-9]{36}",      // GitHub OAuth
        r"|AKIA[A-Z0-9]{16}",         // AWS access key
        r"|sk_live_[a-zA-Z0-9]{20,}", // Stripe live
        r"|sk_test_[a-zA-Z0-9]{20,}", // Stripe test
        r"|xoxb-[a-zA-Z0-9-]{20,}",   // Slack bot token
        r"|xoxp-[a-zA-Z0-9-]{20,}",   // Slack user token
        r"|glpat-[a-zA-Z0-9_-]{20,}", // GitLab PAT
        r"|gxp_[a-zA-Z0-9]{20,}",     // Grafana
        r")",
    ))
    .expect("invalid API_KEY_PATTERNS regex")
});

/// Scrub credential-like content from text before it reaches the LLM.
///
/// `known_values` contains service names and known credential fragments to
/// redact. Values shorter than 4 characters are skipped to avoid false
/// positives (e.g., redacting "the" everywhere).
pub fn scrub(text: &str, known_values: &[&str]) -> String {
    let mut result = text.to_string();

    // 1. Redact known values (passwords, usernames, service names)
    for val in known_values {
        if !val.is_empty() && val.len() > 3 {
            result = result.replace(val, "[REDACTED]");
        }
    }

    // 2. Redact sensitive URL parameters
    result = SENSITIVE_URL_PARAMS
        .replace_all(&result, "$1=[REDACTED]")
        .to_string();

    // 3. Redact auth headers
    result = AUTH_HEADER
        .replace_all(&result, "$1: [REDACTED]")
        .to_string();

    // 4. Redact API key patterns
    result = API_KEY_PATTERNS
        .replace_all(&result, "[REDACTED_KEY]")
        .to_string();

    result
}

/// Matches high-entropy token-like strings (20+ alphanumeric chars).
static HIGH_ENTROPY_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9+/=_\-]{20,}").expect("invalid HIGH_ENTROPY_TOKEN regex")
});

/// UUID pattern — high entropy but not a secret.
static UUID_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
        .expect("invalid UUID_PATTERN regex")
});

/// Home directory patterns — redact usernames from paths.
static HOME_PATH_UNIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)/(?:Users|home)/[^/\s]+/").expect("invalid HOME_PATH regex"));

static HOME_PATH_WIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[A-Z]:\\Users\\[^\\]+\\").expect("invalid HOME_PATH_WIN regex")
});

static IP_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}").expect("invalid IP_PATTERN regex")
});

/// Shannon entropy of a string.
fn shannon_entropy(s: &str) -> f64 {
    let len = s.len() as f64;
    if len == 0.0 {
        return 0.0;
    }
    let mut freq = [0u32; 256];
    for b in s.bytes() {
        freq[b as usize] += 1;
    }
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// Extended scrubbing for bug reports — strips paths with usernames and IPs.
pub fn scrub_for_report(text: &str, known_values: &[&str]) -> String {
    let mut result = scrub(text, known_values);

    // Redact home directory paths
    result = HOME_PATH_UNIX.replace_all(&result, "~/").to_string();
    result = HOME_PATH_WIN.replace_all(&result, r"~\").to_string();

    // Redact IP addresses
    result = IP_PATTERN.replace_all(&result, "[REDACTED_IP]").to_string();

    result
}

/// Entropy-based secret detection for unknown token formats.
///
/// Catches high-entropy strings that regex-based scrubbing misses.
/// Uses TruffleHog/detect-secrets thresholds: 4.5 bits for base64/alphanumeric, 3.0 for hex.
/// Applied only to bug report text (not all outbound messages).
pub fn entropy_scrub(text: &str) -> String {
    // Cap input to prevent excessive scanning
    let capped = if text.len() > 65536 {
        &text[..text
            .char_indices()
            .take_while(|(i, _)| *i <= 65536)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0)]
    } else {
        text
    };

    let mut result = capped.to_string();

    for m in HIGH_ENTROPY_TOKEN.find_iter(capped) {
        let candidate = m.as_str();

        // Skip UUIDs
        if UUID_PATTERN.is_match(candidate) {
            continue;
        }

        let entropy = shannon_entropy(candidate);
        let len = candidate.len();

        // Hex-only strings: lower threshold (3.0)
        let is_hex = candidate
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == '_');
        let threshold = if is_hex { 3.0 } else { 4.5 };
        let min_len = if is_hex { 20 } else { 30 };

        if entropy >= threshold && len >= min_len {
            result = result.replace(candidate, "[REDACTED_HIGH_ENTROPY]");
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_known_password() {
        let text = "Logged in with password MyS3cretP@ss! to the dashboard";
        let result = scrub(text, &["MyS3cretP@ss!"]);
        assert!(
            !result.contains("MyS3cretP@ss!"),
            "Password should be redacted"
        );
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn scrub_known_username() {
        let text = "Welcome, admin@example.com! Your session is active.";
        let result = scrub(text, &["admin@example.com"]);
        assert!(
            !result.contains("admin@example.com"),
            "Username should be redacted"
        );
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn scrub_url_token_param() {
        let text = "Redirected to https://example.com/callback?token=abc123def456&next=/home";
        let result = scrub(text, &[]);
        assert!(
            !result.contains("abc123def456"),
            "Token param should be redacted"
        );
        assert!(result.contains("token=[REDACTED]"));
        assert!(
            result.contains("next=/home"),
            "Non-sensitive params preserved"
        );
    }

    #[test]
    fn scrub_url_api_key_param() {
        let text = "GET /api?api_key=sk_live_abcdef123456&page=1";
        let result = scrub(text, &[]);
        assert!(result.contains("api_key=[REDACTED]"));
        assert!(result.contains("page=1"));
    }

    #[test]
    fn scrub_url_password_param() {
        let text = "https://host/login?password=hunter2&user=bob";
        let result = scrub(text, &[]);
        assert!(result.contains("password=[REDACTED]"));
    }

    #[test]
    fn scrub_url_access_token_param() {
        let text = "oauth?access_token=ya29.long_token_value_here";
        let result = scrub(text, &[]);
        assert!(result.contains("access_token=[REDACTED]"));
    }

    #[test]
    fn scrub_url_session_id_param() {
        let text = "https://app.com/page?session_id=sess_abc123&view=main";
        let result = scrub(text, &[]);
        assert!(result.contains("session_id=[REDACTED]"));
    }

    #[test]
    fn scrub_auth_header_bearer() {
        let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature";
        let result = scrub(text, &[]);
        assert!(
            !result.contains("eyJhbGciOiJIUzI1NiJ9"),
            "Bearer token should be redacted"
        );
        assert!(result.contains("Authorization: [REDACTED]"));
    }

    #[test]
    fn scrub_auth_header_basic() {
        let text = "authorization: Basic dXNlcjpwYXNz";
        let result = scrub(text, &[]);
        assert!(!result.contains("dXNlcjpwYXNz"));
        assert!(result.contains("authorization: [REDACTED]"));
    }

    #[test]
    fn scrub_x_api_key_header() {
        let text = "x-api-key: my-secret-api-key-12345";
        let result = scrub(text, &[]);
        assert!(result.contains("x-api-key: [REDACTED]"));
    }

    #[test]
    fn scrub_x_auth_token_header() {
        let text = "X-Auth-Token: tok_live_abcdef12345678";
        let result = scrub(text, &[]);
        assert!(result.contains("X-Auth-Token: [REDACTED]"));
    }

    #[test]
    fn scrub_openai_api_key() {
        let text = "Found key: sk-proj-abcdefghijklmnopqrstuvwx in the config";
        let result = scrub(text, &[]);
        assert!(
            !result.contains("sk-proj-abcdefghijklmnopqrstuvwx"),
            "OpenAI key should be redacted"
        );
        assert!(result.contains("[REDACTED_KEY]"));
    }

    #[test]
    fn scrub_github_pat() {
        let text = "Token: ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let result = scrub(text, &[]);
        assert!(result.contains("[REDACTED_KEY]"));
    }

    #[test]
    fn scrub_github_oauth() {
        let text = "OAuth: gho_abcdefghijklmnopqrstuvwxyz0123456789";
        let result = scrub(text, &[]);
        assert!(result.contains("[REDACTED_KEY]"));
    }

    #[test]
    fn scrub_preserves_non_sensitive_text() {
        let text = "Welcome to the dashboard. Your name is displayed above.";
        let result = scrub(text, &[]);
        assert_eq!(result, text, "Non-sensitive text should be unchanged");
    }

    #[test]
    fn scrub_short_known_values_skipped() {
        let text = "The cat sat on the mat";
        let result = scrub(text, &["cat"]);
        assert_eq!(
            result, text,
            "Known values of 3 chars or less should be skipped"
        );
    }

    #[test]
    fn scrub_empty_known_values_skipped() {
        let text = "Hello world";
        let result = scrub(text, &[""]);
        assert_eq!(result, text);
    }

    #[test]
    fn scrub_multiple_known_values() {
        let text = "user admin@test.com logged in with password hunter2hunter2";
        let result = scrub(text, &["admin@test.com", "hunter2hunter2"]);
        assert!(!result.contains("admin@test.com"));
        assert!(!result.contains("hunter2hunter2"));
    }

    #[test]
    fn scrub_combined_patterns() {
        let text = "URL: https://api.com?token=abc123&key=xyz789\n\
                     Authorization: Bearer eyJ_token_here\n\
                     API key found: sk-abcdefghijklmnopqrstuvwxyz";
        let result = scrub(text, &[]);
        assert!(result.contains("token=[REDACTED]"));
        assert!(result.contains("key=[REDACTED]"));
        assert!(result.contains("Authorization: [REDACTED]"));
        assert!(result.contains("[REDACTED_KEY]"));
    }

    #[test]
    fn scrub_case_insensitive_url_params() {
        let text = "URL: https://example.com?TOKEN=secret123&PASSWORD=pass456";
        let result = scrub(text, &[]);
        assert!(result.contains("TOKEN=[REDACTED]"));
        assert!(result.contains("PASSWORD=[REDACTED]"));
    }

    #[test]
    fn scrub_csrf_and_nonce_params() {
        let text = "form?csrf=abc123def&nonce=xyz789ghi";
        let result = scrub(text, &[]);
        assert!(result.contains("csrf=[REDACTED]"));
        assert!(result.contains("nonce=[REDACTED]"));
    }

    #[test]
    fn scrub_empty_text() {
        let result = scrub("", &["password"]);
        assert_eq!(result, "");
    }

    #[test]
    fn scrub_known_value_appears_multiple_times() {
        let text = "secret123 was used. Also secret123 appeared again.";
        let result = scrub(text, &["secret123"]);
        assert!(!result.contains("secret123"));
        // Should have two [REDACTED] occurrences
        assert_eq!(result.matches("[REDACTED]").count(), 2);
    }

    // ── scrub_for_report tests ──

    #[test]
    fn scrub_for_report_redacts_unix_home_paths() {
        let text = "/Users/john/Documents/Github/skyclaw/src/main.rs:42";
        let result = scrub_for_report(text, &[]);
        assert_eq!(result, "~/Documents/Github/skyclaw/src/main.rs:42");
    }

    #[test]
    fn scrub_for_report_redacts_linux_home_paths() {
        let text = "/home/developer/project/file.rs:10";
        let result = scrub_for_report(text, &[]);
        assert_eq!(result, "~/project/file.rs:10");
    }

    #[test]
    fn scrub_for_report_redacts_ip() {
        let text = "Connected to 192.168.1.100:8080";
        let result = scrub_for_report(text, &[]);
        assert!(result.contains("[REDACTED_IP]"));
        assert!(!result.contains("192.168"));
    }

    // ── entropy_scrub tests ──

    #[test]
    fn entropy_catches_unknown_api_key() {
        // High-entropy alphanumeric string (>4.5 bits, >30 chars)
        let text = "token: xK9mR2pL5wQ8nJ3vF6hT1yU4sA7dG0bE9cW2iO5kN8jM";
        let result = entropy_scrub(text);
        assert!(
            result.contains("[REDACTED_HIGH_ENTROPY]"),
            "High-entropy token should be redacted, got: {}",
            result
        );
    }

    #[test]
    fn entropy_preserves_normal_text() {
        let text = "The quick brown fox jumps over the lazy dog";
        let result = entropy_scrub(text);
        assert_eq!(result, text);
    }

    #[test]
    fn entropy_preserves_short_strings() {
        let text = "commit: abc123def456";
        let result = entropy_scrub(text);
        assert!(result.contains("abc123def456"));
    }

    #[test]
    fn entropy_preserves_file_paths() {
        let text = "at crates/temm1e-agent/src/runtime.rs:407";
        let result = entropy_scrub(text);
        // Path segments are short — should not trigger
        assert!(result.contains("temm1e-agent"));
    }

    #[test]
    fn shannon_entropy_empty() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    #[test]
    fn shannon_entropy_single_char() {
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    fn shannon_entropy_high_for_random() {
        // 26 unique chars — high entropy
        let high = "abcdefghijklmnopqrstuvwxyz";
        assert!(shannon_entropy(high) > 4.0);
    }
}
