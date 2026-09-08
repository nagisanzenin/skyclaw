//! Credential management — loading, saving, and detecting API keys.
//!
//! Extracted from `main.rs` so that both the CLI binary and `temm1e-tui`
//! can share credential logic without duplication.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing;

use crate::types::error::Temm1eError;

// ── Data Types ──────────────────────────────────────────────────────

/// Credentials file layout (multi-provider, multi-key).
///
/// ```toml
/// active = "anthropic"
///
/// [[providers]]
/// name = "anthropic"
/// keys = ["sk-ant-key1", "sk-ant-key2"]
/// model = "claude-sonnet-4-6"
/// ```
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct CredentialsFile {
    /// Name of the currently active provider.
    #[serde(default)]
    pub active: String,
    /// All configured providers.
    #[serde(default)]
    pub providers: Vec<CredentialsProvider>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CredentialsProvider {
    pub name: String,
    #[serde(default)]
    pub keys: Vec<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// Result of credential detection from user input.
#[derive(Clone)]
pub struct DetectedCredential {
    pub provider: &'static str,
    pub api_key: String,
    pub base_url: Option<String>,
    /// User-specified model name for proxy flows (e.g. `model:qwen3-coder`).
    /// Populated only by `parse_proxy_config` when the user explicitly passes
    /// a `model:` / `m:` / `default_model:` k/v pair. Raw-paste auto-detect
    /// paths always leave this `None` so the onboarding flow falls back to
    /// the provider's hardcoded default via `default_model()`.
    pub model: Option<String>,
}

impl std::fmt::Debug for CredentialsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialsProvider")
            .field("name", &self.name)
            .field("model", &self.model)
            .field("key_count", &self.keys.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DetectedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetectedCredential")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

// ── Path Helpers ────────────────────────────────────────────────────

/// Returns `~/.temm1e/credentials.toml`.
pub fn credentials_path() -> PathBuf {
    crate::config::data_dir().join("credentials.toml")
}

// ── Placeholder Detection ───────────────────────────────────────────

/// Reject obviously fake / placeholder API keys before they reach any provider.
pub fn is_placeholder_key(key: &str) -> bool {
    let k = key.trim().to_lowercase();
    if k.len() < 10 {
        return true;
    }
    let placeholders = [
        "paste_your",
        "your_key",
        "your_api",
        "your-key",
        "your-api",
        "insert_your",
        "insert-your",
        "put_your",
        "put-your",
        "replace_with",
        "replace-with",
        "enter_your",
        "enter-your",
        "placeholder",
        "xxxxxxxx",
        "your_token",
        "your-token",
        "_here",
    ];
    for p in &placeholders {
        if k.contains(p) {
            return true;
        }
    }
    // All same character (e.g. "aaaaaaaaaa")
    if k.len() >= 10 && k.chars().all(|c| c == k.chars().next().unwrap_or('a')) {
        return true;
    }
    false
}

/// Lenient placeholder check for custom-endpoint providers (proxy / `base_url` set).
///
/// LM Studio, Ollama, vLLM and other local inference servers ignore the API key
/// entirely. Users routinely set keys shorter than 10 chars (e.g. `sk-lm-xxx`,
/// `lm-studio`, or even a single character). Strict mode wrongly rejects these
/// because its 10-char minimum was designed for first-party providers.
///
/// Lenient mode removes ONLY the length gate — every other protection from
/// strict mode is preserved:
/// - empty / whitespace-only keys → rejected
/// - known copy-paste markers (`YOUR_API_KEY`, `paste_your_key`, …) → rejected
/// - all-same-char padding (`aaaa`, `0000`) → rejected
///
/// Callers MUST gate lenient vs strict on `base_url.is_some()`. Raw-paste
/// auto-detect paths (no `proxy` keyword, no custom base_url) must keep using
/// strict `is_placeholder_key` — the 10-char guard still earns its keep there.
pub fn is_placeholder_key_lenient(key: &str) -> bool {
    let k = key.trim().to_lowercase();
    if k.is_empty() {
        return true;
    }
    let placeholders = [
        "paste_your",
        "your_key",
        "your_api",
        "your-key",
        "your-api",
        "insert_your",
        "insert-your",
        "put_your",
        "put-your",
        "replace_with",
        "replace-with",
        "enter_your",
        "enter-your",
        "placeholder",
        "xxxxxxxx",
        "your_token",
        "your-token",
        "_here",
    ];
    for p in &placeholders {
        if k.contains(p) {
            return true;
        }
    }
    // All-same-char padding — lenient keeps this guard but at a lower length
    // threshold (4) since strict mode's 10-char gate is removed entirely.
    if k.len() >= 4 && k.chars().all(|c| c == k.chars().next().unwrap_or('a')) {
        return true;
    }
    false
}

// ── Provider Name Normalization ─────────────────────────────────────

/// Normalize provider name string to a static str.
pub fn normalize_provider_name(name: &str) -> Option<&'static str> {
    match name.to_lowercase().as_str() {
        "anthropic" | "claude" => Some("anthropic"),
        "openai" | "gpt" => Some("openai"),
        "gemini" | "google" => Some("gemini"),
        "grok" | "xai" => Some("grok"),
        "openrouter" => Some("openrouter"),
        "minimax" => Some("minimax"),
        "stepfun" | "step" => Some("stepfun"),
        "zai" | "zhipu" | "glm" => Some("zai"),
        "zai-coding-plan" | "glm-coding-plan" => Some("zai-coding-plan"),
        "ollama" => Some("ollama"),
        "lmstudio" | "lm-studio" | "lm_studio" => Some("lmstudio"),
        _ => None,
    }
}

// ── API Key Detection ───────────────────────────────────────────────

/// Detect API provider from user input. Supports multiple formats:
///
/// 1. Raw key (auto-detect): `sk-ant-xxx`
/// 2. Explicit provider:key: `minimax:eyJhbG...`
/// 3. Proxy config: `proxy provider:openai base_url:https://my-proxy/v1 key:sk-xxx`
pub fn detect_api_key(text: &str) -> Option<DetectedCredential> {
    let trimmed = text.trim();

    // Format 3: Proxy config
    let lower = trimmed.to_lowercase();
    if lower.starts_with("proxy") {
        let result = parse_proxy_config(trimmed);
        if let Some(ref cred) = result {
            // Proxy keys use lenient mode — LM Studio / Ollama / vLLM users
            // legitimately use short keys. Empty, whitespace, copy-paste
            // markers, and padding are still rejected.
            let is_placeholder = if cred.base_url.is_some() {
                is_placeholder_key_lenient(&cred.api_key)
            } else {
                is_placeholder_key(&cred.api_key)
            };
            if is_placeholder {
                return None;
            }
        }
        return result;
    }

    // Format 2: Explicit provider:key
    if let Some((provider, key)) = trimmed.split_once(':') {
        let p = provider.to_lowercase();
        if p != "http" && p != "https" {
            match p.as_str() {
                "anthropic" | "openai" | "gemini" | "grok" | "xai" | "openrouter" | "minimax"
                | "stepfun" | "step" | "zai" | "zhipu" | "ollama" | "lmstudio" | "lm-studio"
                | "zai-coding-plan" | "glm-coding-plan" | "github" | "gh"
                    if key.len() >= 8 && !is_placeholder_key(key) =>
                {
                    return Some(DetectedCredential {
                        provider: match p.as_str() {
                            "anthropic" => "anthropic",
                            "openai" => "openai",
                            "gemini" => "gemini",
                            "grok" | "xai" => "grok",
                            "openrouter" => "openrouter",
                            "minimax" => "minimax",
                            "stepfun" | "step" => "stepfun",
                            "zai" | "zhipu" => "zai",
                            "zai-coding-plan" | "glm-coding-plan" => "zai-coding-plan",
                            "ollama" => "ollama",
                            "lmstudio" | "lm-studio" => "lmstudio",
                            "github" | "gh" => "github",
                            _ => unreachable!(),
                        },
                        api_key: key.to_string(),
                        base_url: None,
                        model: None,
                    });
                }
                _ => {}
            }
        }
    }

    // Format 1: Auto-detect from key prefix
    if is_placeholder_key(trimmed) {
        return None;
    }
    if trimmed.starts_with("ghp_") || trimmed.starts_with("github_pat_") {
        return Some(DetectedCredential {
            provider: "github",
            api_key: trimmed.to_string(),
            base_url: None,
            model: None,
        });
    }
    if trimmed.starts_with("sk-ant-") {
        Some(DetectedCredential {
            provider: "anthropic",
            api_key: trimmed.to_string(),
            base_url: None,
            model: None,
        })
    } else if trimmed.starts_with("sk-or-") {
        Some(DetectedCredential {
            provider: "openrouter",
            api_key: trimmed.to_string(),
            base_url: None,
            model: None,
        })
    } else if trimmed.starts_with("xai-") {
        Some(DetectedCredential {
            provider: "grok",
            api_key: trimmed.to_string(),
            base_url: None,
            model: None,
        })
    } else if trimmed.starts_with("sk-") {
        Some(DetectedCredential {
            provider: "openai",
            api_key: trimmed.to_string(),
            base_url: None,
            model: None,
        })
    } else if trimmed.starts_with("AIzaSy") {
        Some(DetectedCredential {
            provider: "gemini",
            api_key: trimmed.to_string(),
            base_url: None,
            model: None,
        })
    } else {
        None
    }
}

/// Parse proxy configuration from user input.
fn parse_proxy_config(text: &str) -> Option<DetectedCredential> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 3 {
        return None;
    }

    let mut provider: Option<&'static str> = None;
    let mut base_url: Option<String> = None;
    let mut api_key: Option<String> = None;
    let mut model: Option<String> = None;

    let mut i = 1;
    while i < tokens.len() {
        let token = tokens[i];
        let lower = token.to_lowercase();

        if let Some((k, v)) = token.split_once(':') {
            let k_lower = k.to_lowercase();
            match k_lower.as_str() {
                "provider" | "type" => {
                    provider = normalize_provider_name(v);
                }
                "base_url" | "url" | "endpoint" | "host" => {
                    base_url = Some(v.to_string());
                }
                "key" | "api_key" | "apikey" | "token" => {
                    api_key = Some(v.to_string());
                }
                "model" | "m" | "default_model" => {
                    // User-specified model for proxy/custom endpoints
                    // (e.g. `model:qwen3-coder-30b-a3b`). Skipping validation
                    // at the onboarding step means this value flows directly
                    // into the agent config without a provider round-trip.
                    model = Some(v.to_string());
                }
                _ => {
                    if v.starts_with("//") || v.starts_with("http") {
                        base_url = Some(token.to_string());
                    } else if normalize_provider_name(&lower).is_some() {
                        provider = normalize_provider_name(k);
                        api_key = Some(v.to_string());
                    }
                }
            }
        } else if token.starts_with("http://") || token.starts_with("https://") {
            base_url = Some(token.to_string());
        } else if normalize_provider_name(&lower).is_some() && provider.is_none() {
            provider = normalize_provider_name(&lower);
        } else if token.len() >= 8 && api_key.is_none() {
            api_key = Some(token.to_string());
        }

        i += 1;
    }

    let provider = provider.unwrap_or("openai");
    let api_key = api_key?;

    // For providers with an implicit local-default endpoint, persist the URL
    // here so credentials.toml is self-describing and v5.3.2's skip condition
    // in `validate_provider_key` (`config.base_url.is_some()`) covers this
    // case automatically. Without this inject, `proxy lmstudio sk-key` (no
    // URL) would produce `base_url = None`, validate would fall through to
    // a real HTTP call, and the factory's injected localhost default would
    // hang for ~120s waiting for a LM Studio instance that may not be
    // running. See temm1e-labs/temm1e#45 (v5.3.3).
    let base_url = base_url.or_else(|| match provider {
        "lmstudio" => Some("http://localhost:1234/v1".to_string()),
        _ => None,
    });

    Some(DetectedCredential {
        provider,
        api_key,
        base_url,
        model,
    })
}

// ── Credential File Operations ──────────────────────────────────────

/// Load the full credentials file. Falls back to legacy single-provider format.
pub fn load_credentials_file() -> Option<CredentialsFile> {
    let path = credentials_path();
    let content = std::fs::read_to_string(&path).ok()?;

    // Try new format first
    if let Ok(creds) = toml::from_str::<CredentialsFile>(&content) {
        if !creds.providers.is_empty() {
            return Some(creds);
        }
    }

    // Fallback: legacy single-provider format
    let table: toml::Table = content.parse().ok()?;
    let provider = table.get("provider")?.as_table()?;
    let name = provider.get("name")?.as_str()?.to_string();
    let key = provider.get("api_key")?.as_str()?.to_string();
    let model = provider.get("model")?.as_str()?.to_string();
    if name.is_empty() || key.is_empty() {
        return None;
    }
    Some(CredentialsFile {
        active: name.clone(),
        providers: vec![CredentialsProvider {
            name,
            keys: vec![key],
            model,
            base_url: None,
        }],
    })
}

/// Save credentials — appends key to existing provider or creates new entry.
pub async fn save_credentials(
    provider_name: &str,
    api_key: &str,
    model: &str,
    custom_base_url: Option<&str>,
) -> Result<(), Temm1eError> {
    let dir = crate::config::data_dir();
    let path = dir.join("credentials.toml");

    let mut creds = load_credentials_file().unwrap_or_default();

    let match_fn = |p: &CredentialsProvider| -> bool {
        p.name == provider_name && p.base_url == custom_base_url.map(|s| s.to_string())
    };

    if let Some(existing) = creds.providers.iter_mut().find(|p| match_fn(p)) {
        if !existing.keys.contains(&api_key.to_string()) {
            existing.keys.push(api_key.to_string());
            tracing::info!(
                provider = %provider_name,
                total_keys = existing.keys.len(),
                "Added new key to existing provider"
            );
        }
        existing.model = model.to_string();
    } else {
        creds.providers.push(CredentialsProvider {
            name: provider_name.to_string(),
            keys: vec![api_key.to_string()],
            model: model.to_string(),
            base_url: custom_base_url.map(|s| s.to_string()),
        });
    }

    creds.active = provider_name.to_string();

    let content = toml::to_string_pretty(&creds)
        .map_err(|e| Temm1eError::Config(format!("Failed to serialize credentials: {e}")))?;
    crate::private_file::write_private_atomic(&path, content.as_bytes())?;
    tracing::info!(path = %path.display(), provider = %provider_name, "Credentials saved");
    Ok(())
}

/// Load the active provider's credentials.
/// Returns `(provider_name, api_key, model)`.
/// Filters out placeholder/dummy keys.
///
/// Uses lenient-mode placeholder check when `provider.base_url.is_some()` so
/// that custom endpoints (LM Studio, Ollama, vLLM, …) can use short keys.
pub fn load_saved_credentials() -> Option<(String, String, String)> {
    let creds = load_credentials_file()?;
    let provider = creds
        .providers
        .iter()
        .find(|p| p.name == creds.active)
        .or_else(|| creds.providers.first())?;
    let has_custom_endpoint = provider.base_url.is_some();
    let first_valid_key = provider
        .keys
        .iter()
        .find(|k| {
            if has_custom_endpoint {
                !is_placeholder_key_lenient(k)
            } else {
                !is_placeholder_key(k)
            }
        })?
        .clone();
    if provider.name.is_empty() || first_valid_key.is_empty() {
        return None;
    }
    Some((
        provider.name.clone(),
        first_valid_key,
        provider.model.clone(),
    ))
}

/// Load all keys for the active provider.
/// Returns `(name, keys, model, base_url)`.
/// Filters out placeholder/dummy keys.
///
/// Uses lenient-mode placeholder check when `provider.base_url.is_some()` so
/// that custom endpoints (LM Studio, Ollama, vLLM, …) can use short keys.
pub fn load_active_provider_keys() -> Option<(String, Vec<String>, String, Option<String>)> {
    let creds = load_credentials_file()?;
    let provider = creds
        .providers
        .iter()
        .find(|p| p.name == creds.active)
        .or_else(|| creds.providers.first())?;
    let has_custom_endpoint = provider.base_url.is_some();
    let valid_keys: Vec<String> = provider
        .keys
        .iter()
        .filter(|k| {
            if has_custom_endpoint {
                !is_placeholder_key_lenient(k)
            } else {
                !is_placeholder_key(k)
            }
        })
        .cloned()
        .collect();
    if provider.name.is_empty() || valid_keys.is_empty() {
        return None;
    }
    Some((
        provider.name.clone(),
        valid_keys,
        provider.model.clone(),
        provider.base_url.clone(),
    ))
}

#[cfg(test)]
mod tests {

    #[test]
    fn credential_debug_does_not_expose_secret_or_endpoint() {
        let credential = super::CredentialsProvider {
            name: "test".into(),
            keys: vec!["private-api-secret".into()],
            model: "test".into(),
            base_url: Some("https://secret-endpoint.invalid".into()),
        };
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("private-api-secret"));
        assert!(!rendered.contains("secret-endpoint"));
    }
    use super::*;

    #[test]
    fn placeholder_keys_rejected() {
        assert!(is_placeholder_key("short"));
        assert!(is_placeholder_key("paste_your_key_here"));
        assert!(is_placeholder_key("aaaaaaaaaa"));
        assert!(is_placeholder_key("YOUR_API_KEY"));
        assert!(is_placeholder_key("placeholder_key"));
    }

    #[test]
    fn real_keys_accepted() {
        assert!(!is_placeholder_key("sk-ant-api03-realkey1234567890"));
        assert!(!is_placeholder_key("sk-proj-realkey1234567890abcdef"));
        assert!(!is_placeholder_key("xai-realkey1234567890"));
    }

    #[test]
    fn detect_anthropic_key() {
        let cred = detect_api_key("sk-ant-api03-abc123").unwrap();
        assert_eq!(cred.provider, "anthropic");
        assert_eq!(cred.api_key, "sk-ant-api03-abc123");
        assert!(cred.base_url.is_none());
    }

    #[test]
    fn detect_openai_key() {
        let cred = detect_api_key("sk-proj-abc123def456").unwrap();
        assert_eq!(cred.provider, "openai");
    }

    #[test]
    fn detect_openrouter_key() {
        let cred = detect_api_key("sk-or-v1-abc123def456").unwrap();
        assert_eq!(cred.provider, "openrouter");
    }

    #[test]
    fn detect_grok_key() {
        let cred = detect_api_key("xai-abc123def456").unwrap();
        assert_eq!(cred.provider, "grok");
    }

    #[test]
    fn detect_gemini_key() {
        let cred = detect_api_key("AIzaSyABCDEF1234567890").unwrap();
        assert_eq!(cred.provider, "gemini");
    }

    #[test]
    fn detect_explicit_provider_key() {
        let cred = detect_api_key("minimax:eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9").unwrap();
        assert_eq!(cred.provider, "minimax");
        assert_eq!(cred.api_key, "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9");
    }

    #[test]
    fn detect_proxy_config() {
        let cred =
            detect_api_key("proxy openai https://my-proxy.com/v1 sk-real-key-12345678").unwrap();
        assert_eq!(cred.provider, "openai");
        assert_eq!(cred.api_key, "sk-real-key-12345678");
        assert_eq!(cred.base_url.as_deref(), Some("https://my-proxy.com/v1"));
    }

    #[test]
    fn detect_proxy_kv_format() {
        let cred = detect_api_key(
            "proxy provider:anthropic base_url:https://gateway.ai/v1 key:sk-ant-api03-real12345678",
        )
        .unwrap();
        assert_eq!(cred.provider, "anthropic");
        assert_eq!(cred.api_key, "sk-ant-api03-real12345678");
        assert_eq!(cred.base_url.as_deref(), Some("https://gateway.ai/v1"));
    }

    #[test]
    fn reject_placeholder_in_detect() {
        assert!(detect_api_key("paste_your_key_here").is_none());
        assert!(detect_api_key("short").is_none());
    }

    #[test]
    fn normalize_known_providers() {
        assert_eq!(normalize_provider_name("anthropic"), Some("anthropic"));
        assert_eq!(normalize_provider_name("claude"), Some("anthropic"));
        assert_eq!(normalize_provider_name("openai"), Some("openai"));
        assert_eq!(normalize_provider_name("gpt"), Some("openai"));
        assert_eq!(normalize_provider_name("gemini"), Some("gemini"));
        assert_eq!(normalize_provider_name("google"), Some("gemini"));
        assert_eq!(normalize_provider_name("grok"), Some("grok"));
        assert_eq!(normalize_provider_name("xai"), Some("grok"));
        assert_eq!(normalize_provider_name("openrouter"), Some("openrouter"));
        assert_eq!(normalize_provider_name("minimax"), Some("minimax"));
        assert_eq!(normalize_provider_name("zai"), Some("zai"));
        assert_eq!(normalize_provider_name("zhipu"), Some("zai"));
        assert_eq!(normalize_provider_name("glm"), Some("zai"));
        assert_eq!(normalize_provider_name("ollama"), Some("ollama"));
        assert_eq!(normalize_provider_name("lmstudio"), Some("lmstudio"));
        assert_eq!(normalize_provider_name("lm-studio"), Some("lmstudio"));
        assert_eq!(normalize_provider_name("lm_studio"), Some("lmstudio"));
        assert_eq!(normalize_provider_name("LMStudio"), Some("lmstudio"));
        assert_eq!(normalize_provider_name("unknown"), None);
    }

    #[test]
    fn parse_proxy_lmstudio_with_explicit_url() {
        // User-provided URL takes precedence and is stored verbatim.
        let cred = detect_api_key("proxy lmstudio http://localhost:1234/v1 sk-anything").unwrap();
        assert_eq!(cred.provider, "lmstudio");
        assert_eq!(cred.api_key, "sk-anything");
        assert_eq!(cred.base_url.as_deref(), Some("http://localhost:1234/v1"));
    }

    #[test]
    fn parse_proxy_lmstudio_without_url_injects_localhost_default() {
        // `proxy lmstudio sk-key` (no URL) — parse_proxy_config must inject
        // the localhost default so credentials.toml is self-describing and
        // v5.3.2's `base_url.is_some()` skip in validate_provider_key covers
        // this case automatically. See #45.
        let cred = detect_api_key("proxy lmstudio sk-anything-key-12345").unwrap();
        assert_eq!(cred.provider, "lmstudio");
        assert_eq!(cred.api_key, "sk-anything-key-12345");
        assert_eq!(
            cred.base_url.as_deref(),
            Some("http://localhost:1234/v1"),
            "lmstudio must get implicit localhost URL at parse time"
        );
    }

    #[test]
    fn parse_proxy_lmstudio_with_remote_url_overrides_default() {
        // A remote LM Studio URL is honored and NOT overwritten by the
        // localhost default. The `or_else` only fires when base_url is None.
        let cred =
            detect_api_key("proxy lmstudio http://10.0.0.5:1234/v1 sk-remote-key-12345").unwrap();
        assert_eq!(cred.provider, "lmstudio");
        assert_eq!(cred.base_url.as_deref(), Some("http://10.0.0.5:1234/v1"));
    }

    #[test]
    fn parse_proxy_lmstudio_dashed_alias_also_injects_default() {
        let cred = detect_api_key("proxy lm-studio sk-anything-key-12345").unwrap();
        assert_eq!(cred.provider, "lmstudio");
        assert_eq!(cred.base_url.as_deref(), Some("http://localhost:1234/v1"));
    }

    #[test]
    fn parse_proxy_other_providers_do_not_get_lmstudio_default() {
        // Regression guard: the or_else must only fire for lmstudio, not
        // for any other provider. `proxy openai sk-key` (no URL) keeps
        // base_url=None as before.
        let cred = detect_api_key("proxy openai sk-test-anything-1234567890").unwrap();
        assert_eq!(cred.provider, "openai");
        assert!(
            cred.base_url.is_none(),
            "non-lmstudio providers must not get the lmstudio default URL"
        );
    }

    #[test]
    fn parse_explicit_lmstudio_prefix() {
        // Format 2: `lmstudio:sk-anything` — explicit provider:key prefix.
        let cred = detect_api_key("lmstudio:sk-anything-key-12345").unwrap();
        assert_eq!(cred.provider, "lmstudio");
        assert_eq!(cred.api_key, "sk-anything-key-12345");
    }

    // ── is_placeholder_key_lenient (Bug 1 fix — temm1e-labs/temm1e#44) ───

    #[test]
    fn lenient_mode_accepts_short_proxy_keys() {
        // The exact literal from bug #44 — 9 chars
        assert!(!is_placeholder_key_lenient("sk-lm-xxx"));
        // Common LM Studio / Ollama convention
        assert!(!is_placeholder_key_lenient("lm-studio"));
        assert!(!is_placeholder_key_lenient("ollama"));
        // Real-world short proxy keys
        assert!(!is_placeholder_key_lenient("token-xyz"));
        assert!(!is_placeholder_key_lenient("abc123"));
        // Edge case: single char
        assert!(!is_placeholder_key_lenient("x"));
    }

    #[test]
    fn lenient_mode_rejects_empty_and_whitespace() {
        assert!(is_placeholder_key_lenient(""));
        assert!(is_placeholder_key_lenient("   "));
        assert!(is_placeholder_key_lenient("\t"));
        assert!(is_placeholder_key_lenient("\n"));
        assert!(is_placeholder_key_lenient("\t\n "));
    }

    #[test]
    fn lenient_mode_still_rejects_copy_paste_markers() {
        // Even proxy users can fat-finger a copy-paste placeholder —
        // lenient mode still catches them.
        assert!(is_placeholder_key_lenient("YOUR_API_KEY"));
        assert!(is_placeholder_key_lenient("YOUR_API_KEY_HERE"));
        assert!(is_placeholder_key_lenient("paste_your_key_here"));
        assert!(is_placeholder_key_lenient("insert_your_api_key"));
        assert!(is_placeholder_key_lenient("replace_with_key"));
        assert!(is_placeholder_key_lenient("put_your_key_here"));
        assert!(is_placeholder_key_lenient("enter_your_key"));
        assert!(is_placeholder_key_lenient("placeholder"));
        assert!(is_placeholder_key_lenient("xxxxxxxx"));
        assert!(is_placeholder_key_lenient("your_token"));
        assert!(is_placeholder_key_lenient("api_key_here"));
    }

    #[test]
    fn lenient_mode_still_rejects_all_same_char_padding() {
        // Short padding
        assert!(is_placeholder_key_lenient("aaaa"));
        assert!(is_placeholder_key_lenient("0000"));
        assert!(is_placeholder_key_lenient("xxxx"));
        // Longer padding (also caught by strict mode)
        assert!(is_placeholder_key_lenient("aaaaaaaaaa"));
    }

    #[test]
    fn lenient_mode_allows_mixed_short_patterns() {
        // 3-char all-same-char is NOT blocked (too short to be useful padding
        // detection, and legitimate tokens can look anything). This is a
        // deliberate trade-off — the length floor for padding detection is 4.
        assert!(!is_placeholder_key_lenient("abc"));
        assert!(!is_placeholder_key_lenient("a1b"));
    }

    #[test]
    fn strict_mode_byte_identical_for_long_keys() {
        // Regression guard: strict mode behavior must NOT change for long keys.
        // Every caller that stays on strict mode sees identical behavior.
        assert!(!is_placeholder_key("sk-ant-api03-realkey1234567890"));
        assert!(!is_placeholder_key("sk-proj-realkey1234567890abcdef"));
        assert!(is_placeholder_key("short")); // still rejected
        assert!(is_placeholder_key("sk-lm-xxx")); // 9 chars still rejected in strict
    }

    // ── detect_api_key proxy flow with lenient mode ──────────────────────

    #[test]
    fn detect_proxy_accepts_short_key_with_base_url() {
        // Reproduces the exact input from temm1e-labs/temm1e#44
        let cred = detect_api_key("proxy openai http://100.100.1.251:1234/v1 sk-lm-xxx").unwrap();
        assert_eq!(cred.provider, "openai");
        assert_eq!(cred.api_key, "sk-lm-xxx");
        assert_eq!(
            cred.base_url.as_deref(),
            Some("http://100.100.1.251:1234/v1")
        );
    }

    #[test]
    fn detect_proxy_accepts_lm_studio_style_key() {
        let cred = detect_api_key("proxy openai http://localhost:1234/v1 lm-studio").unwrap();
        assert_eq!(cred.api_key, "lm-studio");
        assert_eq!(cred.base_url.as_deref(), Some("http://localhost:1234/v1"));
    }

    #[test]
    fn detect_proxy_rejects_empty_key_even_with_base_url() {
        // Parser would need a key token — empty string isn't reachable via the
        // whitespace-split path, but the k:v form can synthesize one.
        assert!(detect_api_key("proxy openai url:http://localhost:1234/v1 key:").is_none());
    }

    #[test]
    fn detect_proxy_rejects_copy_paste_marker_even_with_base_url() {
        // Lenient mode still catches obvious placeholder markers.
        assert!(detect_api_key("proxy openai http://localhost:1234/v1 YOUR_API_KEY").is_none());
        assert!(
            detect_api_key("proxy openai http://localhost:1234/v1 paste_your_key_here").is_none()
        );
    }

    #[test]
    fn detect_raw_paste_still_rejects_short_key_without_proxy_keyword() {
        // Strict mode still applies when there's no `proxy` keyword — the
        // copy-paste guard earns its keep on the raw-paste auto-detect path.
        assert!(detect_api_key("sk-lm-xxx").is_none());
        assert!(detect_api_key("short").is_none());
        assert!(detect_api_key("lm-studio").is_none());
    }

    // ── Q2: proxy `model:` parameter (temm1e-labs/temm1e#44 Phase 2) ─────

    #[test]
    fn parse_proxy_config_with_model_kv() {
        let cred = detect_api_key(
            "proxy openai http://localhost:1234/v1 sk-lm-xxx model:qwen3-coder-30b-a3b",
        )
        .unwrap();
        assert_eq!(cred.provider, "openai");
        assert_eq!(cred.api_key, "sk-lm-xxx");
        assert_eq!(cred.base_url.as_deref(), Some("http://localhost:1234/v1"));
        assert_eq!(cred.model.as_deref(), Some("qwen3-coder-30b-a3b"));
    }

    #[test]
    fn parse_proxy_config_with_m_shorthand() {
        let cred =
            detect_api_key("proxy m:llama-3.3 url:http://localhost:11434/v1 key:ollama").unwrap();
        assert_eq!(cred.model.as_deref(), Some("llama-3.3"));
        assert_eq!(cred.base_url.as_deref(), Some("http://localhost:11434/v1"));
        assert_eq!(cred.api_key, "ollama");
    }

    #[test]
    fn parse_proxy_config_with_default_model_alias() {
        let cred = detect_api_key("proxy openai http://lm/v1 sk-lm-xxx default_model:qwen3-coder")
            .unwrap();
        assert_eq!(cred.model.as_deref(), Some("qwen3-coder"));
    }

    #[test]
    fn parse_proxy_config_model_is_optional() {
        // Legacy call without model: still works — cred.model == None means
        // the onboarding flow falls back to default_model(provider).
        let cred = detect_api_key("proxy openai http://localhost:1234/v1 sk-lm-xxx").unwrap();
        assert!(cred.model.is_none(), "no model: key means None");
        assert_eq!(cred.api_key, "sk-lm-xxx");
    }

    #[test]
    fn raw_paste_leaves_model_none() {
        // Auto-detect paths (no proxy keyword) never populate model — they
        // always fall through to default_model(provider) in onboarding.
        let cred = detect_api_key("sk-ant-api03-realkey1234567890").unwrap();
        assert!(cred.model.is_none());

        let cred = detect_api_key("sk-proj-abc123def456ghi789").unwrap();
        assert!(cred.model.is_none());

        let cred = detect_api_key("AIzaSyABCDEF1234567890").unwrap();
        assert!(cred.model.is_none());

        let cred = detect_api_key("xai-abc123def456").unwrap();
        assert!(cred.model.is_none());
    }

    #[test]
    fn parse_proxy_config_model_field_is_first_class_for_bug_44_input() {
        // Reproduces the exact input the user would type in the fixed workflow
        // for temm1e-labs/temm1e#44.
        let cred = detect_api_key(
            "proxy openai http://100.100.1.251:1234/v1 sk-lm-xxx model:qwen3-coder-30b-a3b",
        )
        .unwrap();
        assert_eq!(cred.provider, "openai");
        assert_eq!(cred.api_key, "sk-lm-xxx");
        assert_eq!(
            cred.base_url.as_deref(),
            Some("http://100.100.1.251:1234/v1")
        );
        assert_eq!(cred.model.as_deref(), Some("qwen3-coder-30b-a3b"));
    }
}
