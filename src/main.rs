use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine as _;
use clap::{Parser, Subcommand};
use futures::FutureExt;
use skyclaw_core::Channel;
use tokio::sync::Mutex;

// ── Secret-censoring channel wrapper ──────────────────────
// Wraps any Channel to censor known API keys from outbound messages.
// This is the hardcoded last-line-of-defense filter — the system prompt
// tells the agent not to leak secrets, but this catches anything that slips.
struct SecretCensorChannel {
    inner: Arc<dyn Channel>,
}

#[async_trait]
impl Channel for SecretCensorChannel {
    fn name(&self) -> &str {
        self.inner.name()
    }
    async fn start(&mut self) -> std::result::Result<(), skyclaw_core::types::error::SkyclawError> {
        Ok(())
    }
    async fn stop(&mut self) -> std::result::Result<(), skyclaw_core::types::error::SkyclawError> {
        Ok(())
    }
    async fn send_message(
        &self,
        mut msg: skyclaw_core::types::message::OutboundMessage,
    ) -> std::result::Result<(), skyclaw_core::types::error::SkyclawError> {
        msg.text = censor_secrets(&msg.text);
        self.inner.send_message(msg).await
    }
    fn file_transfer(&self) -> Option<&dyn skyclaw_core::FileTransfer> {
        self.inner.file_transfer()
    }
    fn is_allowed(&self, user_id: &str) -> bool {
        self.inner.is_allowed(user_id)
    }
    async fn delete_message(
        &self,
        chat_id: &str,
        message_id: &str,
    ) -> std::result::Result<(), skyclaw_core::types::error::SkyclawError> {
        self.inner.delete_message(chat_id, message_id).await
    }
}

#[derive(Parser)]
#[command(name = "skyclaw")]
#[command(about = "Cloud-native Rust AI agent runtime — Telegram-native")]
#[command(version)]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Runtime mode: cloud, local, or auto
    #[arg(long, default_value = "auto")]
    mode: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the SkyClaw gateway daemon
    Start,
    /// Interactive CLI chat with the agent
    Chat,
    /// Show gateway status, connected channels, provider health
    Status,
    /// Manage skills
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
    /// Manage configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Show version information
    Version,
}

#[derive(Subcommand)]
enum SkillCommands {
    /// List installed skills
    List,
    /// Show skill details
    Info { name: String },
    /// Install a skill from a path
    Install { path: String },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Validate the current configuration
    Validate,
    /// Show resolved configuration
    Show,
}

// ── Onboarding helpers ─────────────────────────────────────

/// Result of credential detection from user input.
struct DetectedCredential {
    provider: &'static str,
    api_key: String,
    base_url: Option<String>,
}

/// Reject obviously fake / placeholder API keys before they reach any provider.
/// This prevents bricking the agent by saving a dummy key to credentials.toml.
fn is_placeholder_key(key: &str) -> bool {
    let k = key.trim().to_lowercase();
    // Too short to be any real API key
    if k.len() < 10 {
        return true;
    }
    // Common placeholders users might paste from docs/examples/READMEs
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
        "_here", // catches PASTE_YOUR_KEY_HERE, PUT_KEY_HERE, etc.
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

/// Validate a provider key by making a minimal API call.
/// Returns Ok(provider_arc) if the key works, Err(message) if not.
async fn validate_provider_key(
    config: &skyclaw_core::types::config::ProviderConfig,
) -> Result<Arc<dyn skyclaw_core::Provider>, String> {
    let provider = skyclaw_providers::create_provider(config)
        .map_err(|e| format!("Failed to create provider: {}", e))?;
    let provider_arc: Arc<dyn skyclaw_core::Provider> = Arc::from(provider);

    let test_req = skyclaw_core::types::message::CompletionRequest {
        model: config.model.clone().unwrap_or_default(),
        messages: vec![skyclaw_core::types::message::ChatMessage {
            role: skyclaw_core::types::message::Role::User,
            content: skyclaw_core::types::message::MessageContent::Text("Hi".to_string()),
        }],
        tools: Vec::new(),
        max_tokens: Some(1),
        temperature: Some(0.0),
        system: None,
    };

    match provider_arc.complete(test_req).await {
        Ok(_) => Ok(provider_arc),
        Err(e) => {
            let err_str = format!("{}", e);
            let err_lower = err_str.to_lowercase();
            // Auth errors or invalid model errors — reject the reload
            if err_lower.contains("401")
                || err_lower.contains("403")
                || err_lower.contains("unauthorized")
                || err_lower.contains("invalid api key")
                || err_lower.contains("invalid x-api-key")
                || err_lower.contains("authentication")
                || err_lower.contains("permission")
                || err_lower.contains("404")
                || err_lower.contains("not_found")
                || err_lower.contains("model:")
            {
                Err(err_str)
            } else {
                // Non-auth errors (400 max_tokens, 429 rate limit, etc.) mean
                // the key IS valid — the API accepted the auth, just rejected
                // the request params. This is fine for validation.
                tracing::debug!(error = %err_str, "Key validation got non-auth error — key is valid");
                Ok(provider_arc)
            }
        }
    }
}

/// Detect API provider from user input. Supports multiple formats:
///
/// 1. Raw key (auto-detect): `sk-ant-xxx`
/// 2. Explicit provider:key: `minimax:eyJhbG...`
/// 3. Proxy config (key:value pairs on one or multiple lines):
///    `proxy provider:openai base_url:https://my-proxy/v1 key:sk-xxx`
///    or `proxy openai https://my-proxy/v1 sk-xxx` (positional shorthand)
fn detect_api_key(text: &str) -> Option<DetectedCredential> {
    let trimmed = text.trim();

    // ── Format 3: Proxy config ──────────────────────────────
    // Detect "proxy" keyword (case-insensitive)
    let lower = trimmed.to_lowercase();
    if lower.starts_with("proxy") {
        let result = parse_proxy_config(trimmed);
        // Validate proxy key isn't a placeholder
        if let Some(ref cred) = result {
            if is_placeholder_key(&cred.api_key) {
                return None;
            }
        }
        return result;
    }

    // ── Format 2: Explicit provider:key ─────────────────────
    if let Some((provider, key)) = trimmed.split_once(':') {
        // Don't match "http:" or "https:" as provider:key
        let p = provider.to_lowercase();
        if p != "http" && p != "https" {
            match p.as_str() {
                "anthropic" | "openai" | "gemini" | "grok" | "xai" | "openrouter" | "minimax"
                | "zai" | "zhipu" | "ollama" => {
                    if key.len() >= 8 && !is_placeholder_key(key) {
                        return Some(DetectedCredential {
                            provider: match p.as_str() {
                                "anthropic" => "anthropic",
                                "openai" => "openai",
                                "gemini" => "gemini",
                                "grok" | "xai" => "grok",
                                "openrouter" => "openrouter",
                                "minimax" => "minimax",
                                "zai" | "zhipu" => "zai",
                                "ollama" => "ollama",
                                _ => unreachable!(),
                            },
                            api_key: key.to_string(),
                            base_url: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // ── Format 1: Auto-detect from key prefix ───────────────
    // Reject placeholders before accepting
    if is_placeholder_key(trimmed) {
        return None;
    }
    if trimmed.starts_with("sk-ant-") {
        Some(DetectedCredential {
            provider: "anthropic",
            api_key: trimmed.to_string(),
            base_url: None,
        })
    } else if trimmed.starts_with("sk-or-") {
        Some(DetectedCredential {
            provider: "openrouter",
            api_key: trimmed.to_string(),
            base_url: None,
        })
    } else if trimmed.starts_with("xai-") {
        Some(DetectedCredential {
            provider: "grok",
            api_key: trimmed.to_string(),
            base_url: None,
        })
    } else if trimmed.starts_with("sk-") {
        Some(DetectedCredential {
            provider: "openai",
            api_key: trimmed.to_string(),
            base_url: None,
        })
    } else if trimmed.starts_with("AIzaSy") {
        Some(DetectedCredential {
            provider: "gemini",
            api_key: trimmed.to_string(),
            base_url: None,
        })
    } else {
        None
    }
}

/// Parse proxy configuration from user input.
///
/// Supports flexible formats:
///   `proxy provider:openai base_url:https://... key:sk-xxx`
///   `proxy provider:openai url:https://... key:sk-xxx`
///   `proxy openai https://my-proxy.com/v1 sk-xxx`  (positional shorthand)
///
/// Also handles multi-line input (Telegram sends line breaks).
fn parse_proxy_config(text: &str) -> Option<DetectedCredential> {
    // Normalize: join all lines, split by whitespace
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 3 {
        return None; // Need at least "proxy <provider> <key>"
    }

    let mut provider: Option<&'static str> = None;
    let mut base_url: Option<String> = None;
    let mut api_key: Option<String> = None;

    // Skip the "proxy" token
    let mut i = 1;
    while i < tokens.len() {
        let token = tokens[i];
        let lower = token.to_lowercase();

        // Key:value format
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
                // Could be a provider:key or url with port
                _ => {
                    if v.starts_with("//") || v.starts_with("http") {
                        // It's a URL like "https://..."
                        base_url = Some(token.to_string());
                    } else if normalize_provider_name(&lower).is_some() {
                        // e.g. "openai:sk-xxx" — treat as provider + key
                        provider = normalize_provider_name(k);
                        api_key = Some(v.to_string());
                    }
                }
            }
        } else if token.starts_with("http://") || token.starts_with("https://") {
            // Positional: bare URL
            base_url = Some(token.to_string());
        } else if normalize_provider_name(&lower).is_some() && provider.is_none() {
            // Positional: provider name
            provider = normalize_provider_name(&lower);
        } else if token.len() >= 8 && api_key.is_none() {
            // Positional: assume it's the API key (long enough token)
            api_key = Some(token.to_string());
        }

        i += 1;
    }

    // Provider defaults to "openai" for proxies (most common use case)
    let provider = provider.unwrap_or("openai");
    let api_key = api_key?;

    Some(DetectedCredential {
        provider,
        api_key,
        base_url,
    })
}

/// Normalize provider name string to a static str.
fn normalize_provider_name(name: &str) -> Option<&'static str> {
    match name.to_lowercase().as_str() {
        "anthropic" | "claude" => Some("anthropic"),
        "openai" | "gpt" => Some("openai"),
        "gemini" | "google" => Some("gemini"),
        "grok" | "xai" => Some("grok"),
        "openrouter" => Some("openrouter"),
        "minimax" => Some("minimax"),
        "zai" | "zhipu" | "glm" => Some("zai"),
        "ollama" => Some("ollama"),
        _ => None,
    }
}

/// Default model for each provider.
fn default_model(provider_name: &str) -> &'static str {
    match provider_name {
        "anthropic" => "claude-sonnet-4-6",
        "openai" => "gpt-5.2",
        "gemini" => "gemini-3-flash-preview",
        "grok" | "xai" => "grok-4-1-fast-non-reasoning",
        "openrouter" => "anthropic/claude-sonnet-4-6",
        "minimax" => "MiniMax-M2.5",
        "zai" => "glm-4.7-flash",
        "ollama" => "llama3.3",
        _ => "claude-sonnet-4-6",
    }
}

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
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct CredentialsFile {
    /// Name of the currently active provider.
    #[serde(default)]
    active: String,
    /// All configured providers.
    #[serde(default)]
    providers: Vec<CredentialsProvider>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CredentialsProvider {
    name: String,
    #[serde(default)]
    keys: Vec<String>,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
}

fn credentials_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".skyclaw")
        .join("credentials.toml")
}

/// Load the full credentials file. Falls back to legacy single-provider format.
fn load_credentials_file() -> Option<CredentialsFile> {
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
/// If `custom_base_url` is provided, it creates a separate proxy entry.
async fn save_credentials(
    provider_name: &str,
    api_key: &str,
    model: &str,
    custom_base_url: Option<&str>,
) -> Result<()> {
    let dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".skyclaw");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("credentials.toml");

    let mut creds = load_credentials_file().unwrap_or_default();

    // For proxy providers with custom base_url, match on name + base_url
    // to keep them separate from the default endpoint entry.
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

    // Set this provider as active
    creds.active = provider_name.to_string();

    let content = toml::to_string_pretty(&creds)?;
    tokio::fs::write(&path, content).await?;
    tracing::info!(path = %path.display(), provider = %provider_name, "Credentials saved");
    Ok(())
}

/// Load the active provider's credentials (backwards-compatible return type).
/// Filters out placeholder/dummy keys — returns None if no valid key exists.
fn load_saved_credentials() -> Option<(String, String, String)> {
    let creds = load_credentials_file()?;
    // Find active provider
    let provider = creds
        .providers
        .iter()
        .find(|p| p.name == creds.active)
        .or_else(|| creds.providers.first())?;
    // Find the first non-placeholder key
    let first_valid_key = provider
        .keys
        .iter()
        .find(|k| !is_placeholder_key(k))?
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
/// Filters out placeholder/dummy keys — returns None if no valid keys remain.
fn load_active_provider_keys() -> Option<(String, Vec<String>, String, Option<String>)> {
    let creds = load_credentials_file()?;
    let provider = creds
        .providers
        .iter()
        .find(|p| p.name == creds.active)
        .or_else(|| creds.providers.first())?;
    // Filter out placeholders
    let valid_keys: Vec<String> = provider
        .keys
        .iter()
        .filter(|k| !is_placeholder_key(k))
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

/// Build the onboarding welcome message with a pre-generated setup link.
fn onboarding_message_with_link(setup_link: &str) -> String {
    format!(
        "Welcome to SkyClaw!\n\n\
         To get started, open this secure setup link:\n\
         {}\n\n\
         Paste your API key in the form, copy the encrypted blob, \
         and send it back here.\n\n\
         Or just paste your API key directly below — \
         I'll auto-detect the provider and get you online.\n\n\
         You can add more keys later with /addkey, \
         list them with /keys, or remove with /removekey.",
        setup_link
    )
}

const ONBOARDING_REFERENCE: &str = "\
Supported formats:\n\n\
1\u{fe0f}\u{20e3} Auto-detect (just paste the key):\n\
sk-ant-...     \u{2192} Anthropic\n\
sk-...         \u{2192} OpenAI\n\
AIzaSy...      \u{2192} Gemini\n\
xai-...        \u{2192} Grok\n\
sk-or-...      \u{2192} OpenRouter\n\n\
2\u{fe0f}\u{20e3} Explicit (for keys without unique prefix):\n\
zai:YOUR_KEY\n\
minimax:YOUR_KEY\n\
openrouter:YOUR_KEY\n\
ollama:YOUR_KEY\n\n\
3\u{fe0f}\u{20e3} Proxy / custom endpoint:\n\
proxy <provider> <base_url> <api_key>\n\n\
Example:\n\
proxy openai https://my-proxy.com/v1 sk-xxx\n\
proxy anthropic https://gateway.ai/v1/anthropic sk-ant-xxx\n\
proxy ollama https://ollama.com/v1 your-ollama-key";

const SYSTEM_PROMPT_BASE: &str = "\
You are SkyClaw, a cloud-native AI agent running on a remote server. \
You have full access to these tools:\n\
- shell: run any command\n\
- file_read / file_write / file_list: filesystem operations\n\
- web_fetch: HTTP GET requests\n\
- browser: control a real Chrome browser (navigate, click, type, screenshot, \
  get_text, evaluate JS, get_html) — use this for any website interaction\n\
- send_message: send real-time messages to the user during tasks\n\
- send_file: send files to the user\n\
- memory_manage: your persistent knowledge store (remember/recall/forget/update/list)\n\n\
KEY RULES:\n\
- Shell output (stdout/stderr) is NOT visible to the user. Only YOUR \
  final text reply and send_message calls reach the user.\n\
- To send real-time updates during multi-step work, call send_message \
  each round. You can do this repeatedly — each call delivers instantly.\n\
- For timed/periodic tasks (e.g. send something every N seconds), use \
  shell(\"sleep N\") then send_message in a loop across tool rounds.\n\
- When asked to visit a website, open a page, or interact with a web app, \
  USE THE BROWSER TOOL. Do not refuse or explain why you can't — just do it.\n\
- After finishing browser work, call browser with action 'close' to shut it down.\n\
- Reply in the same language the user writes in.\n\
- Be concise. No emoji unless the user uses them.\n\
- NEVER give up on a task by explaining limitations. You have a multi-round \
  tool loop — keep calling tools until the task is done or you hit a real \
  error. Do not stop early to explain what you 'cannot' do.\n\n\
PERSISTENT MEMORY:\n\
You have a persistent knowledge store via the memory_manage tool. Use it to:\n\
- Remember important facts the user tells you (name, preferences, project details)\n\
- Save useful context that should persist across conversations\n\
- Recall previously saved knowledge when relevant to the conversation\n\
When to use memory_manage:\n\
- When the user explicitly asks you to remember something\n\
- When you learn an important fact about the user or their project\n\
- When the user corrects you — update the relevant memory\n\
- When you need context from a previous conversation\n\
CRITICAL: After EVERY remember/update/forget action, you MUST tell the user \
what you did. For example: 'I've remembered that your name is Alice' or \
'I've updated the project status to completed' or 'I've forgotten the old API endpoint'. \
Never silently save or delete memories.";

/// Build the full system prompt with dynamic provider/model context.
/// This ensures the bot always knows what's actually configured.
fn build_system_prompt() -> String {
    let mut prompt = SYSTEM_PROMPT_BASE.to_string();

    // ── Provider/model context ────────────────────────────────
    prompt.push_str("\n\nSUPPORTED PROVIDERS & DEFAULT MODELS:\n");
    prompt.push_str("- anthropic: claude-sonnet-4-6, claude-opus-4-6, claude-haiku-4-6\n");
    prompt.push_str("- openai: gpt-5.2, gpt-4.1, gpt-4.1-mini, o4-mini\n");
    prompt.push_str("- gemini: gemini-3-flash-preview, gemini-3.1-pro-preview, gemini-2.5-flash, gemini-2.5-pro\n");
    prompt.push_str("- grok (xai): grok-4-1-fast-non-reasoning, grok-3\n");
    prompt.push_str(
        "- openrouter: any model via anthropic/claude-sonnet-4-6, openai/gpt-5.2, etc.\n",
    );
    prompt.push_str("- zai (zhipu): glm-4.7-flash, glm-4.7, glm-5, glm-5-code, glm-4.6v\n");
    prompt.push_str("- minimax: MiniMax-M2.5\n");

    // ── Vision capability ──────────────────────────────────────
    prompt.push_str(
        "\nVISION (IMAGE) SUPPORT:\n\
         Models that can see images: all claude-*, all gpt-4o/gpt-4.1/gpt-5.*, all gemini-*, \
         grok-3/grok-4, glm-*v* (V-suffix only, e.g. glm-4.6v-flash).\n\
         Text-only (NO vision): gpt-3.5-turbo, glm-4.7-flash, glm-4.7, glm-5, glm-5-code, \
         glm-4.5-flash, all MiniMax models.\n\
         If the user sends an image on a text-only model, images are auto-stripped and \
         the user is notified. Suggest switching to a vision model.\n",
    );

    // ── Current configuration ─────────────────────────────────
    if let Some(creds) = load_credentials_file() {
        prompt.push_str("\nCURRENT CONFIGURATION:\n");
        prompt.push_str(&format!("Active provider: {}\n", creds.active));
        for p in &creds.providers {
            let key_count = p.keys.iter().filter(|k| !is_placeholder_key(k)).count();
            let base_note = if let Some(ref url) = p.base_url {
                format!(" (via {})", url)
            } else {
                String::new()
            };
            prompt.push_str(&format!(
                "- {}: model={}, {} key(s){}\n",
                p.name, p.model, key_count, base_note
            ));
        }
    }

    // ── Self-configuration rules ──────────────────────────────
    prompt.push_str(
        "\n\
SELF-CONFIGURATION:\n\
Your config lives at ~/.skyclaw/credentials.toml.\n\
To change the active provider or model, edit ONLY the 'active' field or 'model' \
field in credentials.toml. NEVER modify or add API keys directly — keys are \
managed by the onboarding system. If the user wants to add a key, tell them to \
paste it in chat.\n\
Changes take effect immediately — SkyClaw validates the key and auto-reloads \
after each response. If a key is invalid, the switch is rejected and the \
current provider stays active.\n\
Users can add keys anytime by pasting them in chat. SkyClaw auto-detects the \
provider and validates before saving.\n\n\
SECRET HANDLING (MANDATORY — NEVER VIOLATE):\n\
There are 3 environments: USER (human) → CLAW (you, the agent) → PC (the server you run on).\n\
- Users give you secrets (API keys, passwords, tokens, account IDs) for YOU to use.\n\
- You ARE allowed to use secrets on the PC: log into services, call APIs, configure tools, \
  do personal tasks for the user. This is your job.\n\
- You must NEVER send secrets BACK to the user in your replies. Secrets flow one way: \
  user → claw. Never claw → user.\n\
- You must NEVER post secrets on the internet (no pasting keys in public repos, \
  web forms, or chat services other than the user's own channel).\n\
Specific rules:\n\
- NEVER echo back an API key the user pasted, not even partially.\n\
- NEVER read credentials.toml and show its contents to the user.\n\
- NEVER include API keys in shell commands visible to the user.\n\
- If the user asks to see their key, say it's stored securely and cannot be displayed.\n\
- When confirming a key was added, say 'Key saved for [provider]' — never show the key.\n\
- This applies to ALL secrets: API keys, tokens, passwords, encrypted blobs, account IDs.\n\
A secondary output filter censors any key that leaks, but you must not rely on it. \
The primary defense is YOU never including secrets in your output.",
    );

    prompt
}

/// Hardcoded output filter: replaces any known API key in the text with [REDACTED].
/// This is the last line of defense — the system prompt tells the agent not to leak
/// secrets, but this filter catches any that slip through.
fn censor_secrets(text: &str) -> String {
    let creds = match load_credentials_file() {
        Some(c) => c,
        None => return text.to_string(),
    };
    let mut censored = text.to_string();
    for provider in &creds.providers {
        for key in &provider.keys {
            if !key.is_empty() && !is_placeholder_key(key) && key.len() >= 8 {
                censored = censored.replace(key, "[REDACTED]");
            }
        }
    }
    censored
}

// ── OTK key management helpers ────────────────────────────

/// List configured providers (names only, never keys).
fn list_configured_providers() -> String {
    match load_credentials_file() {
        Some(creds) => {
            if creds.providers.is_empty() {
                return "No providers configured. Use /addkey to add one.".to_string();
            }
            let mut lines = vec!["Configured providers:".to_string()];
            for p in &creds.providers {
                let key_count = p.keys.iter().filter(|k| !is_placeholder_key(k)).count();
                let active = if p.name == creds.active {
                    " (active)"
                } else {
                    ""
                };
                let proxy = if let Some(ref url) = p.base_url {
                    format!(" via {}", url)
                } else {
                    String::new()
                };
                lines.push(format!(
                    "  {} — model: {}, {} key(s){}{}",
                    p.name, p.model, key_count, proxy, active
                ));
            }
            lines.push(String::new());
            lines.push(
                "Use /addkey to add a new key, /removekey <provider> to remove one.".to_string(),
            );
            lines.join("\n")
        }
        None => "No providers configured. Use /addkey to add one.".to_string(),
    }
}

/// Handle the /model command.
///
/// - `/model` (no args) → show current model + all available models per provider
/// - `/model <exact-name>` → switch to that model on the active provider
fn handle_model_command(args: &str) -> String {
    let creds = match load_credentials_file() {
        Some(c) => c,
        None => return "No providers configured. Use /addkey to add one.".to_string(),
    };

    if creds.providers.is_empty() {
        return "No providers configured. Use /addkey to add one.".to_string();
    }

    // ── No args: show current + available models ──────────────
    if args.is_empty() {
        let mut lines = Vec::new();

        // Current model
        if let Some(active) = creds.providers.iter().find(|p| p.name == creds.active) {
            lines.push(format!(
                "Current: {} on {} provider",
                active.model, active.name
            ));
        }

        lines.push(String::new());
        lines.push("Available models per provider:".to_string());
        for p in &creds.providers {
            let models = available_models_for_provider(&p.name);
            let active_marker = if p.name == creds.active {
                " (active)"
            } else {
                ""
            };
            let is_proxy = p.base_url.is_some() || p.name == "openrouter";
            lines.push(format!("  {}{}:", p.name, active_marker));
            if is_proxy {
                let current_vision = if is_vision_model(&p.model) {
                    " [vision]"
                } else {
                    ""
                };
                lines.push(format!("    {} ← current{}", p.model, current_vision));
                lines.push("    (proxy — any model name accepted)".to_string());
            } else {
                for m in &models {
                    let vision = if is_vision_model(m) { " [vision]" } else { "" };
                    let current = if *m == p.model { " ← current" } else { "" };
                    lines.push(format!("    {}{}{}", m, vision, current));
                }
            }
        }

        lines.push(String::new());
        lines.push("Switch model: /model <exact-model-name>".to_string());
        lines.push("Example: /model claude-sonnet-4-6".to_string());
        return lines.join("\n");
    }

    // ── Switch to specific model ──────────────────────────────
    let target = args.trim();

    // Find active provider
    let active_provider = match creds.providers.iter().find(|p| p.name == creds.active) {
        Some(p) => p.clone(),
        None => return "Active provider not found in credentials.".to_string(),
    };

    if active_provider.model == target {
        return format!("Already using {}.", target);
    }

    // Validate model against known list for the active provider.
    // Skip validation for proxy/OpenRouter providers (custom base_url) — they accept any model.
    let is_proxy = active_provider.base_url.is_some() || active_provider.name == "openrouter";
    let known = available_models_for_provider(&active_provider.name);
    if !is_proxy && !known.is_empty() && !known.contains(&target) {
        let list = known
            .iter()
            .map(|m| {
                let v = if is_vision_model(m) { " [vision]" } else { "" };
                format!("  {}{}", m, v)
            })
            .collect::<Vec<_>>()
            .join("\n");
        return format!(
            "Unknown model '{}' for provider '{}'.\n\nAvailable models:\n{}\n\nUse exact name: /model <model-name>",
            target, active_provider.name, list
        );
    }

    // Update the model in credentials.toml
    let mut updated = creds.clone();
    for p in &mut updated.providers {
        if p.name == creds.active {
            p.model = target.to_string();
        }
    }

    let path = credentials_path();
    match toml::to_string_pretty(&updated) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, &content) {
                return format!("Failed to write credentials: {}", e);
            }
            tracing::info!(
                old_model = %active_provider.model,
                new_model = %target,
                "Model switched via /model command"
            );
            format!(
                "Model switched: {} → {}\nHot-reload will apply after this response.",
                active_provider.model, target
            )
        }
        Err(e) => format!("Failed to serialize credentials: {}", e),
    }
}

/// Known models for each provider (used by /model listing).
fn available_models_for_provider(provider: &str) -> Vec<&'static str> {
    match provider {
        "anthropic" => vec!["claude-sonnet-4-6", "claude-opus-4-6", "claude-haiku-4-5"],
        "openai" => vec![
            "gpt-5.2",
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4o",
            "o4-mini",
            "o3-mini",
            "gpt-3.5-turbo",
        ],
        "gemini" => vec![
            "gemini-3-flash-preview",
            "gemini-3.1-pro-preview",
            "gemini-2.5-flash",
            "gemini-2.5-pro",
        ],
        "grok" | "xai" => vec!["grok-4-1-fast-non-reasoning", "grok-3"],
        "openrouter" => vec![
            "anthropic/claude-sonnet-4-6",
            "openai/gpt-5.2",
            "google/gemini-3-flash-preview",
        ],
        "zai" | "zhipu" => vec![
            "glm-4.7-flash",
            "glm-4.7",
            "glm-5",
            "glm-5-code",
            "glm-4.6v",
            "glm-4.6v-flash",
        ],
        "minimax" => vec!["MiniMax-M2.5", "MiniMax-M2.5-highspeed"],
        _ => vec![],
    }
}

/// Quick vision check for /model display (mirrors runtime::model_supports_vision).
fn is_vision_model(model: &str) -> bool {
    let m = model.to_lowercase();
    if m.starts_with("glm-") {
        return m.contains('v') && !m.starts_with("glm-5");
    }
    if m.starts_with("minimax") {
        return false;
    }
    if m.starts_with("gpt-3") {
        return false;
    }
    true
}

/// Remove a provider from credentials.
fn remove_provider(provider_name: &str) -> String {
    if provider_name.is_empty() {
        return "Usage: /removekey <provider>\nExample: /removekey openai".to_string();
    }
    let mut creds = match load_credentials_file() {
        Some(c) => c,
        None => return "No providers configured.".to_string(),
    };
    let before = creds.providers.len();
    creds.providers.retain(|p| p.name != provider_name);
    if creds.providers.len() == before {
        return format!(
            "Provider '{}' not found. Use /keys to see configured providers.",
            provider_name
        );
    }
    // If we removed the active provider, switch to first remaining
    if creds.active == provider_name {
        creds.active = creds
            .providers
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_default();
    }
    let path = credentials_path();
    match toml::to_string_pretty(&creds) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                return format!("Failed to save: {}", e);
            }
        }
        Err(e) => return format!("Failed to serialize: {}", e),
    }
    if creds.providers.is_empty() {
        format!(
            "Removed {}. No providers remaining — send a new API key to configure one.",
            provider_name
        )
    } else {
        format!(
            "Removed {}. Active provider: {} (model: {})",
            provider_name,
            creds.active,
            creds
                .providers
                .first()
                .map(|p| p.model.as_str())
                .unwrap_or("unknown")
        )
    }
}

/// Decrypt an `enc:v1:` blob using the OTK from the setup token store.
async fn decrypt_otk_blob(
    blob_b64: &str,
    store: &skyclaw_gateway::SetupTokenStore,
    chat_id: &str,
) -> std::result::Result<String, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    // Look up OTK for this chat
    let otk = store
        .consume(chat_id)
        .await
        .ok_or_else(|| "No pending setup link for this chat. Run /addkey first.".to_string())?;

    // Base64 decode
    let blob = base64::engine::general_purpose::STANDARD
        .decode(blob_b64.trim())
        .map_err(|e| format!("Invalid base64: {}", e))?;

    // Need at least 12 (IV) + 16 (tag) + 1 (ciphertext) bytes
    if blob.len() < 29 {
        return Err("Encrypted blob too short.".to_string());
    }

    // Split: first 12 bytes = IV, rest = ciphertext + auth tag
    let (iv_bytes, ciphertext) = blob.split_at(12);

    let key = Key::<Aes256Gcm>::from_slice(&otk);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(iv_bytes);

    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        "Decryption failed — the setup link may have expired or the data was tampered with."
            .to_string()
    })?;

    String::from_utf8(plaintext).map_err(|_| "Decrypted data is not valid UTF-8.".to_string())
}

// ── Stop-command detection ─────────────────────────────────
fn is_stop_command(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    const STOP_WORDS: &[&str] = &[
        // English
        "stop",
        "cancel",
        "abort",
        "quit",
        "halt",
        "enough",
        // Vietnamese
        "dừng",
        "dung",
        "thôi",
        "thoi",
        "ngừng",
        "ngung",
        "hủy",
        "huy",
        "dẹp",
        "dep",
        // Spanish
        "para",
        "detente",
        "basta",
        "cancela",
        "alto",
        // French
        "arrête",
        "arrete",
        "arrêter",
        "arreter",
        "annuler",
        "suffit",
        // German
        "stopp",
        "aufhören",
        "aufhoren",
        "abbrechen",
        "genug",
        // Portuguese
        "pare",
        "parar",
        "cancele",
        "cancelar",
        "chega",
        // Italian
        "ferma",
        "fermati",
        "basta",
        "annulla",
        "smettila",
        // Russian
        "стоп",
        "стой",
        "хватит",
        "отмена",
        "довольно",
        // Japanese
        "止めて",
        "やめて",
        "やめろ",
        "ストップ",
        "止め",
        "やめ",
        // Korean
        "멈춰",
        "그만",
        "중지",
        "취소",
        "됐어",
        // Chinese
        "停",
        "停止",
        "取消",
        "别说了",
        "够了",
        "算了",
        // Arabic
        "توقف",
        "الغاء",
        "كفى",
        "قف",
        // Thai
        "หยุด",
        "ยกเลิก",
        "พอ",
        "เลิก",
        // Indonesian / Malay
        "berhenti",
        "hentikan",
        "batalkan",
        "cukup",
        "sudah",
        // Hindi
        "रुको",
        "बंद",
        "रद्द",
        "बस",
        "ruko",
        "bas",
        // Turkish
        "dur",
        "durdur",
        "iptal",
        "yeter",
    ];

    if STOP_WORDS.contains(&t.as_str()) {
        return true;
    }

    if t.len() <= 60 {
        const STOP_PHRASES: &[&str] = &[
            "stop it",
            "stop that",
            "please stop",
            "stop now",
            "cancel that",
            "shut up",
            "dừng lại",
            "dung lai",
            "thôi đi",
            "thoi di",
            "dừng đi",
            "dung di",
            "ngừng lại",
            "ngung lai",
            "dung viet",
            "dừng viết",
            "thoi dung",
            "thôi dừng",
            "đừng nói nữa",
            "dung noi nua",
            "im đi",
            "im di",
            "para ya",
            "deja de",
            "arrête ça",
            "arrete ca",
            "hör auf",
            "hor auf",
            "止めてください",
            "やめてください",
            "停下来",
            "不要说了",
            "别说了",
            "그만해",
            "멈춰줘",
        ];

        for phrase in STOP_PHRASES {
            if t.contains(phrase) {
                return true;
            }
        }
    }

    false
}

/// Retry `send_message` up to 3 times with exponential backoff.
async fn send_with_retry(
    sender: &dyn skyclaw_core::Channel,
    reply: skyclaw_core::types::message::OutboundMessage,
) {
    let mut attempt = 0u32;
    let msg = reply;
    loop {
        attempt += 1;
        match sender.send_message(msg.clone()).await {
            Ok(_) => return,
            Err(e) => {
                if attempt >= 3 {
                    tracing::error!(error = %e, attempt, "Failed to send reply after 3 attempts — message lost");
                    return;
                }
                tracing::warn!(error = %e, attempt, "Failed to send reply, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(500 * (1 << attempt))).await;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    // Initialize health endpoint uptime clock
    skyclaw_gateway::health::init_start_time();

    // ── Global panic hook — route panics through tracing ─────
    // Without this, panics only write to stderr and are invisible in structured logs.
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else {
            "unknown panic payload".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        tracing::error!(
            panic.payload = %payload,
            panic.location = %location,
            "PANIC caught — task will attempt recovery"
        );
    }));

    // Load configuration
    let config_path = cli.config.as_ref().map(std::path::Path::new);
    let config = skyclaw_core::config::load_config(config_path)?;

    tracing::info!(mode = %cli.mode, "SkyClaw starting");

    match cli.command {
        Commands::Start => {
            tracing::info!("Starting SkyClaw gateway");

            // ── Resolve API credentials ────────────────────────
            // Priority: config file > saved credentials > onboarding
            let credentials: Option<(String, String, String)> = {
                if let Some(ref key) = config.provider.api_key {
                    if !key.is_empty() && !key.starts_with("${") {
                        let name = config
                            .provider
                            .name
                            .clone()
                            .unwrap_or_else(|| "anthropic".to_string());
                        let model = config
                            .provider
                            .model
                            .clone()
                            .unwrap_or_else(|| default_model(&name).to_string());
                        Some((name, key.clone(), model))
                    } else {
                        load_saved_credentials()
                    }
                } else {
                    load_saved_credentials()
                }
            };

            // ── Memory backend ─────────────────────────────────
            let memory_url = config.memory.path.clone().unwrap_or_else(|| {
                let data_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".skyclaw");
                if let Err(e) = std::fs::create_dir_all(&data_dir) {
                    tracing::warn!(error = %e, path = %data_dir.display(), "Failed to create directory");
                }
                format!("sqlite:{}/memory.db?mode=rwc", data_dir.display())
            });
            let memory: Arc<dyn skyclaw_core::Memory> = Arc::from(
                skyclaw_memory::create_memory_backend(&config.memory.backend, &memory_url).await?,
            );
            tracing::info!(backend = %config.memory.backend, "Memory initialized");

            // ── Telegram channel ───────────────────────────────
            let mut channels: Vec<Arc<dyn skyclaw_core::Channel>> = Vec::new();
            let mut primary_channel: Option<Arc<dyn skyclaw_core::Channel>> = None;
            let mut tg_rx: Option<
                tokio::sync::mpsc::Receiver<skyclaw_core::types::message::InboundMessage>,
            > = None;

            if let Some(tg_config) = config.channel.get("telegram") {
                if tg_config.enabled {
                    let mut tg = skyclaw_channels::TelegramChannel::new(tg_config)?;
                    tg.start().await?;
                    tg_rx = tg.take_receiver();
                    let tg_arc: Arc<dyn skyclaw_core::Channel> = Arc::new(tg);
                    channels.push(tg_arc.clone());
                    primary_channel = Some(tg_arc.clone());
                    tracing::info!("Telegram channel started");
                }
            }

            // ── Pending messages ───────────────────────────────
            let pending_messages: skyclaw_tools::PendingMessages =
                Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

            // ── OTK setup token store ───────────────────────────
            let setup_tokens = skyclaw_gateway::SetupTokenStore::new();

            // ── Pending raw key pastes (from /addkey unsafe) ────
            let pending_raw_keys: Arc<Mutex<HashSet<String>>> =
                Arc::new(Mutex::new(HashSet::new()));

            // ── Usage store (shares same SQLite DB as memory) ────
            let usage_store: Arc<dyn skyclaw_core::UsageStore> =
                Arc::new(skyclaw_memory::SqliteUsageStore::new(&memory_url).await?);
            tracing::info!("Usage store initialized");

            // ── Tools (with secret-censoring channel wrapper) ───
            let censored_channel: Option<Arc<dyn Channel>> = primary_channel
                .clone()
                .map(|ch| Arc::new(SecretCensorChannel { inner: ch }) as Arc<dyn Channel>);
            let tools = skyclaw_tools::create_tools(
                &config.tools,
                censored_channel,
                Some(pending_messages.clone()),
                Some(memory.clone()),
                Some(Arc::new(setup_tokens.clone()) as Arc<dyn skyclaw_core::SetupLinkGenerator>),
                Some(usage_store.clone()),
            );
            tracing::info!(count = tools.len(), "Tools initialized");

            let system_prompt = Some(build_system_prompt());

            // ── Agent state (None during onboarding) ───────────
            let agent_state: Arc<tokio::sync::RwLock<Option<Arc<skyclaw_agent::AgentRuntime>>>> =
                Arc::new(tokio::sync::RwLock::new(None));

            if let Some((ref pname, ref key, ref model)) = credentials {
                // Filter out placeholder/invalid keys at startup
                if is_placeholder_key(key) {
                    tracing::warn!(provider = %pname, "Primary API key is a placeholder — starting in onboarding mode");
                    // Fall through to onboarding
                } else {
                    // Load all keys and saved base_url for this provider
                    let (all_keys, saved_base_url) = load_active_provider_keys()
                        .map(|(_, keys, _, burl)| {
                            let valid: Vec<String> = keys
                                .into_iter()
                                .filter(|k| !is_placeholder_key(k))
                                .collect();
                            (valid, burl)
                        })
                        .unwrap_or_else(|| (vec![key.clone()], None));
                    let effective_base_url =
                        saved_base_url.or_else(|| config.provider.base_url.clone());
                    let provider_config = skyclaw_core::types::config::ProviderConfig {
                        name: Some(pname.clone()),
                        api_key: Some(key.clone()),
                        keys: all_keys,
                        model: Some(model.clone()),
                        base_url: effective_base_url,
                        extra_headers: config.provider.extra_headers.clone(),
                    };
                    let provider: Arc<dyn skyclaw_core::Provider> =
                        Arc::from(skyclaw_providers::create_provider(&provider_config)?);
                    let agent = Arc::new(
                        skyclaw_agent::AgentRuntime::with_limits(
                            provider.clone(),
                            memory.clone(),
                            tools.clone(),
                            model.clone(),
                            system_prompt.clone(),
                            config.agent.max_turns,
                            config.agent.max_context_tokens,
                            config.agent.max_tool_rounds,
                            config.agent.max_task_duration_secs,
                            config.agent.max_spend_usd,
                        )
                        .with_v2_optimizations(config.agent.v2_optimizations),
                    );
                    *agent_state.write().await = Some(agent);
                    tracing::info!(provider = %pname, model = %model, "Agent initialized");
                }
            } else {
                tracing::info!("No API key — starting in onboarding mode");
            }

            // ── Unified message channel ────────────────────────
            let (msg_tx, mut msg_rx) =
                tokio::sync::mpsc::channel::<skyclaw_core::types::message::InboundMessage>(32);

            // Track spawned task handles for graceful shutdown
            let mut task_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

            // Wire Telegram messages into the unified channel
            if let Some(mut tg_rx) = tg_rx {
                let tx = msg_tx.clone();
                task_handles.push(tokio::spawn(async move {
                    while let Some(msg) = tg_rx.recv().await {
                        if tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                }));
            }

            // ── Workspace ──────────────────────────────────────
            let workspace_path = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".skyclaw")
                .join("workspace");
            if let Err(e) = std::fs::create_dir_all(&workspace_path) {
                tracing::warn!(error = %e, path = %workspace_path.display(), "Failed to create directory");
            }

            // ── Heartbeat ──────────────────────────────────────
            if config.heartbeat.enabled {
                let heartbeat_chat_id = config
                    .heartbeat
                    .report_to
                    .clone()
                    .unwrap_or_else(|| "heartbeat".to_string());
                let runner = skyclaw_automation::HeartbeatRunner::new(
                    config.heartbeat.clone(),
                    workspace_path.clone(),
                    heartbeat_chat_id,
                );
                let hb_tx = msg_tx.clone();
                task_handles.push(tokio::spawn(async move {
                    runner.run(hb_tx).await;
                }));
                tracing::info!(
                    interval = %config.heartbeat.interval,
                    checklist = %config.heartbeat.checklist,
                    "Heartbeat runner started"
                );
            }

            // ── Per-chat serial executor ───────────────────────

            /// Tracks the active task state for a single chat.
            struct ChatSlot {
                tx: tokio::sync::mpsc::Sender<skyclaw_core::types::message::InboundMessage>,
                interrupt: Arc<AtomicBool>,
                is_heartbeat: Arc<AtomicBool>,
            }

            if let Some(sender) = primary_channel.clone() {
                let agent_state_clone = agent_state.clone();
                let memory_clone = memory.clone();
                let tools_clone = tools.clone();
                let agent_max_turns = config.agent.max_turns;
                let agent_max_context_tokens = config.agent.max_context_tokens;
                let agent_max_tool_rounds = config.agent.max_tool_rounds;
                let agent_max_task_duration = config.agent.max_task_duration_secs;
                let agent_max_spend_usd = config.agent.max_spend_usd;
                let agent_v2_opt = config.agent.v2_optimizations;
                let provider_base_url = config.provider.base_url.clone();
                let ws_path = workspace_path.clone();
                let pending_clone = pending_messages.clone();
                let setup_tokens_clone = setup_tokens.clone();
                let pending_raw_keys_clone = pending_raw_keys.clone();
                let usage_store_clone = usage_store.clone();

                let chat_slots: Arc<Mutex<HashMap<String, ChatSlot>>> =
                    Arc::new(Mutex::new(HashMap::new()));

                let msg_tx_redispatch = msg_tx.clone();
                task_handles.push(tokio::spawn(async move {
                    while let Some(inbound) = msg_rx.recv().await {
                        let chat_id = inbound.chat_id.clone();
                        let is_heartbeat_msg = inbound.channel == "heartbeat";

                        let mut slots = chat_slots.lock().await;

                        // Handle user messages while a task is active
                        if !is_heartbeat_msg {
                            if let Some(slot) = slots.get(&chat_id) {
                                if slot.is_heartbeat.load(Ordering::Relaxed) {
                                    tracing::info!(
                                        chat_id = %chat_id,
                                        "User message preempting active heartbeat task"
                                    );
                                    slot.interrupt.store(true, Ordering::Relaxed);
                                }

                                let is_stop = inbound
                                    .text
                                    .as_deref()
                                    .map(is_stop_command)
                                    .unwrap_or(false);

                                if is_stop {
                                    tracing::info!(
                                        chat_id = %chat_id,
                                        "Stop command detected — interrupting active task"
                                    );
                                    slot.interrupt.store(true, Ordering::Relaxed);
                                    continue;
                                }

                                if let Some(text) = inbound.text.as_deref() {
                                    if let Ok(mut pq) = pending_clone.lock() {
                                        pq.entry(chat_id.clone())
                                            .or_default()
                                            .push(text.to_string());
                                    }
                                }
                            }
                        }

                        // Skip heartbeat if chat is busy
                        if is_heartbeat_msg {
                            if let Some(slot) = slots.get(&chat_id) {
                                if slot.tx.try_send(inbound).is_err() {
                                    tracing::debug!(
                                        chat_id = %chat_id,
                                        "Skipping heartbeat — chat is busy"
                                    );
                                }
                                continue;
                            }
                        }

                        // Ensure a worker exists for this chat_id
                        let slot = slots.entry(chat_id.clone()).or_insert_with(|| {
                            let (chat_tx, mut chat_rx) =
                                tokio::sync::mpsc::channel::<skyclaw_core::types::message::InboundMessage>(4);

                            let interrupt = Arc::new(AtomicBool::new(false));
                            let is_heartbeat = Arc::new(AtomicBool::new(false));

                            let agent_state = agent_state_clone.clone();
                            let memory = memory_clone.clone();
                            let tools_template = tools_clone.clone();
                            let max_turns = agent_max_turns;
                            let max_ctx = agent_max_context_tokens;
                            let max_rounds = agent_max_tool_rounds;
                            let max_task_duration = agent_max_task_duration;
                            let max_spend = agent_max_spend_usd;
                            let v2_opt = agent_v2_opt;
                            let base_url = provider_base_url.clone();
                            let sender = sender.clone();
                            let workspace_path = ws_path.clone();
                            let interrupt_clone = interrupt.clone();
                            let is_heartbeat_clone = is_heartbeat.clone();
                            let pending_for_worker = pending_clone.clone();
                            let setup_tokens_worker = setup_tokens_clone.clone();
                            let pending_raw_keys_worker = pending_raw_keys_clone.clone();
                            let usage_store_worker = usage_store_clone.clone();
                            let worker_chat_id = chat_id.clone();

                            tokio::spawn(async move {
                                // ── Restore conversation history from memory backend ──
                                let history_key = format!("chat_history:{}", worker_chat_id);
                                let mut persistent_history: Vec<skyclaw_core::types::message::ChatMessage> =
                                    match memory.get(&history_key).await {
                                        Ok(Some(entry)) => {
                                            match serde_json::from_str(&entry.content) {
                                                Ok(h) => {
                                                    tracing::info!(
                                                        chat_id = %worker_chat_id,
                                                        messages = %Vec::<skyclaw_core::types::message::ChatMessage>::len(&h),
                                                        "Restored conversation history from memory"
                                                    );
                                                    h
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        chat_id = %worker_chat_id,
                                                        error = %e,
                                                        "Failed to deserialize saved history, starting fresh"
                                                    );
                                                    Vec::new()
                                                }
                                            }
                                        }
                                        Ok(None) => Vec::new(),
                                        Err(e) => {
                                            tracing::warn!(
                                                chat_id = %worker_chat_id,
                                                error = %e,
                                                "Failed to load saved history, starting fresh"
                                            );
                                            Vec::new()
                                        }
                                    };

                                while let Some(mut msg) = chat_rx.recv().await {
                                    // Snapshot for outer panic handler (msg is borrowed by async block)
                                    let panic_chat_id = msg.chat_id.clone();
                                    let panic_msg_id = msg.id.clone();

                                    let outer_catch_result = AssertUnwindSafe(async {
                                    let is_hb = msg.channel == "heartbeat";
                                    is_heartbeat_clone.store(is_hb, Ordering::Relaxed);
                                    interrupt_clone.store(false, Ordering::Relaxed);

                                    let interrupt_flag = Some(interrupt_clone.clone());

                                    // ── Commands — intercepted before agent ──────
                                    let msg_text_cmd = msg.text.as_deref().unwrap_or("");
                                    let cmd_lower = msg_text_cmd.trim().to_lowercase();

                                    // /addkey — secure OTK flow
                                    if cmd_lower == "/addkey" {
                                        let otk = setup_tokens_worker.generate(&msg.chat_id).await;
                                        let otk_hex = hex::encode(otk);
                                        let link = format!(
                                            "https://nagisanzenin.github.io/skyclaw/setup#{}",
                                            otk_hex
                                        );
                                        let reply = skyclaw_core::types::message::OutboundMessage {
                                            chat_id: msg.chat_id.clone(),
                                            text: format!(
                                                "Secure key setup:\n\n\
                                                 1. Open this link:\n{}\n\n\
                                                 2. Paste your API key in the form\n\
                                                 3. Copy the encrypted blob\n\
                                                 4. Paste it back here\n\n\
                                                 Link expires in 10 minutes.\n\n\
                                                 For a quick (less secure) method: /addkey unsafe",
                                                link
                                            ),
                                            reply_to: Some(msg.id.clone()),
                                            parse_mode: None,
                                        };
                                        send_with_retry(&*sender, reply).await;
                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        return;
                                    }

                                    // /addkey unsafe — raw key paste mode
                                    if cmd_lower == "/addkey unsafe" {
                                        pending_raw_keys_worker.lock().await.insert(msg.chat_id.clone());
                                        let reply = skyclaw_core::types::message::OutboundMessage {
                                            chat_id: msg.chat_id.clone(),
                                            text: "Paste your API key in the next message.\n\n\
                                                   Warning: the key will be visible in chat history.\n\
                                                   For a secure method, use /addkey instead."
                                                .to_string(),
                                            reply_to: Some(msg.id.clone()),
                                            parse_mode: None,
                                        };
                                        send_with_retry(&*sender, reply).await;
                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        return;
                                    }

                                    // /keys — list configured providers
                                    if cmd_lower == "/keys" {
                                        let info = list_configured_providers();
                                        let reply = skyclaw_core::types::message::OutboundMessage {
                                            chat_id: msg.chat_id.clone(),
                                            text: info,
                                            reply_to: Some(msg.id.clone()),
                                            parse_mode: None,
                                        };
                                        send_with_retry(&*sender, reply).await;
                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        return;
                                    }

                                    // /model [model-name] — list or switch models
                                    if cmd_lower == "/model" || cmd_lower.starts_with("/model ") {
                                        let args = if cmd_lower == "/model" {
                                            ""
                                        } else {
                                            msg_text_cmd.trim()["/model".len()..].trim()
                                        };
                                        let result = handle_model_command(args);
                                        let is_switch = result.starts_with("Model switched:");

                                        // If model was switched, reload agent immediately
                                        // (don't wait for file watcher)
                                        let final_text = if is_switch {
                                            if let Some(creds) = load_credentials_file() {
                                                if let Some(prov) = creds.providers.iter().find(|p| p.name == creds.active) {
                                                    let valid_keys: Vec<String> = prov.keys.iter()
                                                        .filter(|k| !is_placeholder_key(k))
                                                        .cloned()
                                                        .collect();
                                                    let effective_base_url = prov.base_url.clone().or_else(|| base_url.clone());
                                                    let reload_config = skyclaw_core::types::config::ProviderConfig {
                                                        name: Some(creds.active.clone()),
                                                        api_key: valid_keys.first().cloned(),
                                                        keys: valid_keys,
                                                        model: Some(prov.model.clone()),
                                                        base_url: effective_base_url,
                                                        extra_headers: std::collections::HashMap::new(),
                                                    };
                                                    match validate_provider_key(&reload_config).await {
                                                        Ok(validated_provider) => {
                                                            let new_agent = Arc::new(skyclaw_agent::AgentRuntime::with_limits(
                                                                validated_provider,
                                                                memory.clone(),
                                                                tools_template.clone(),
                                                                prov.model.clone(),
                                                                Some(build_system_prompt()),
                                                                max_turns,
                                                                max_ctx,
                                                                max_rounds,
                                                                max_task_duration,
                                                                max_spend,
                                                            ).with_v2_optimizations(v2_opt));
                                                            *agent_state.write().await = Some(new_agent);
                                                            tracing::info!(
                                                                provider = %creds.active,
                                                                model = %prov.model,
                                                                "Agent reloaded via /model command"
                                                            );
                                                            format!("{}\nActive now.", result)
                                                        }
                                                        Err(err) => {
                                                            tracing::warn!(error = %err, "Model switch failed validation");
                                                            // Revert credentials
                                                            if let Some(old_agent) = agent_state.read().await.as_ref() {
                                                                let old_model = old_agent.model().to_string();
                                                                let mut rev = creds.clone();
                                                                for p in &mut rev.providers {
                                                                    if p.name == creds.active {
                                                                        p.model = old_model.clone();
                                                                    }
                                                                }
                                                                if let Ok(content) = toml::to_string_pretty(&rev) {
                                                                    let _ = std::fs::write(credentials_path(), &content);
                                                                }
                                                            }
                                                            format!("Model switch failed: {}\nReverted to previous model.", err)
                                                        }
                                                    }
                                                } else {
                                                    result
                                                }
                                            } else {
                                                result
                                            }
                                        } else {
                                            result
                                        };

                                        let reply = skyclaw_core::types::message::OutboundMessage {
                                            chat_id: msg.chat_id.clone(),
                                            text: final_text,
                                            reply_to: Some(msg.id.clone()),
                                            parse_mode: None,
                                        };
                                        send_with_retry(&*sender, reply).await;
                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        return;
                                    }

                                    // /removekey <provider>
                                    if cmd_lower.starts_with("/removekey") {
                                        let provider_arg = msg_text_cmd.trim()["/removekey".len()..].trim();
                                        let result = remove_provider(provider_arg);
                                        let reply = skyclaw_core::types::message::OutboundMessage {
                                            chat_id: msg.chat_id.clone(),
                                            text: result,
                                            reply_to: Some(msg.id.clone()),
                                            parse_mode: None,
                                        };
                                        send_with_retry(&*sender, reply).await;

                                        // If provider was removed, check if agent needs to go offline
                                        if !provider_arg.is_empty() && load_active_provider_keys().is_none() {
                                            *agent_state.write().await = None;
                                            tracing::info!("All providers removed — agent offline");
                                        }

                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        return;
                                    }

                                    // /usage — show usage summary
                                    if cmd_lower == "/usage" {
                                        let summary_text = match usage_store_worker.usage_summary(&msg.chat_id).await {
                                            Ok(summary) => {
                                                if summary.turn_count == 0 {
                                                    "No usage records for this chat yet.".to_string()
                                                } else {
                                                    format!(
                                                        "Usage Summary\nTurns: {}\nAPI Calls: {}\nInput Tokens: {}\nOutput Tokens: {}\nCombined Tokens: {}\nTools Used: {}\nTotal Cost: ${:.4}",
                                                        summary.turn_count,
                                                        summary.total_api_calls,
                                                        summary.total_input_tokens,
                                                        summary.total_output_tokens,
                                                        summary.combined_tokens(),
                                                        summary.total_tools_used,
                                                        summary.total_cost_usd,
                                                    )
                                                }
                                            }
                                            Err(e) => format!("Failed to query usage: {}", e),
                                        };
                                        let reply = skyclaw_core::types::message::OutboundMessage {
                                            chat_id: msg.chat_id.clone(),
                                            text: summary_text,
                                            reply_to: Some(msg.id.clone()),
                                            parse_mode: None,
                                        };
                                        send_with_retry(&*sender, reply).await;
                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        return;
                                    }

                                    // enc:v1: — encrypted blob from OTK flow
                                    if msg_text_cmd.trim().starts_with("enc:v1:") {
                                        let blob_b64 = &msg_text_cmd.trim()["enc:v1:".len()..];
                                        match decrypt_otk_blob(blob_b64, &setup_tokens_worker, &msg.chat_id).await {
                                            Ok(api_key_text) => {
                                                // Treat the decrypted text as an API key
                                                if let Some(cred) = detect_api_key(&api_key_text) {
                                                    let model = default_model(cred.provider).to_string();
                                                    let effective_base_url = cred.base_url.clone().or_else(|| base_url.clone());
                                                    let test_config = skyclaw_core::types::config::ProviderConfig {
                                                        name: Some(cred.provider.to_string()),
                                                        api_key: Some(cred.api_key.clone()),
                                                        keys: vec![cred.api_key.clone()],
                                                        model: Some(model.clone()),
                                                        base_url: effective_base_url,
                                                        extra_headers: std::collections::HashMap::new(),
                                                    };
                                                    match validate_provider_key(&test_config).await {
                                                        Ok(validated_provider) => {
                                                            if let Err(e) = save_credentials(cred.provider, &cred.api_key, &model, cred.base_url.as_deref()).await {
                                                                tracing::error!(error = %e, "Failed to save credentials from OTK flow");
                                                            }
                                                            let new_agent = Arc::new(skyclaw_agent::AgentRuntime::with_limits(
                                                                validated_provider,
                                                                memory.clone(),
                                                                tools_template.clone(),
                                                                model.clone(),
                                                                Some(build_system_prompt()),
                                                                max_turns,
                                                                max_ctx,
                                                                max_rounds,
                                                                max_task_duration,
                                                                max_spend,
                                                            ).with_v2_optimizations(v2_opt));
                                                            *agent_state.write().await = Some(new_agent);
                                                            let reply = skyclaw_core::types::message::OutboundMessage {
                                                                chat_id: msg.chat_id.clone(),
                                                                text: format!(
                                                                    "API key securely received and verified! Configured {} with model {}.\n\nSkyClaw is online.",
                                                                    cred.provider, model
                                                                ),
                                                                reply_to: Some(msg.id.clone()),
                                                                parse_mode: None,
                                                            };
                                                            send_with_retry(&*sender, reply).await;
                                                            tracing::info!(provider = %cred.provider, "OTK key validated — agent online");
                                                        }
                                                        Err(err) => {
                                                            let reply = skyclaw_core::types::message::OutboundMessage {
                                                                chat_id: msg.chat_id.clone(),
                                                                text: format!(
                                                                    "Key decrypted but validation failed — {} returned:\n{}\n\nCheck the key and try /addkey again.",
                                                                    cred.provider, err
                                                                ),
                                                                reply_to: Some(msg.id.clone()),
                                                                parse_mode: None,
                                                            };
                                                            send_with_retry(&*sender, reply).await;
                                                        }
                                                    }
                                                } else {
                                                    let reply = skyclaw_core::types::message::OutboundMessage {
                                                        chat_id: msg.chat_id.clone(),
                                                        text: "Decrypted successfully but couldn't detect the provider. \
                                                               Make sure you pasted a valid API key in the setup page."
                                                            .to_string(),
                                                        reply_to: Some(msg.id.clone()),
                                                        parse_mode: None,
                                                    };
                                                    send_with_retry(&*sender, reply).await;
                                                }
                                            }
                                            Err(err) => {
                                                let reply = skyclaw_core::types::message::OutboundMessage {
                                                    chat_id: msg.chat_id.clone(),
                                                    text: err,
                                                    reply_to: Some(msg.id.clone()),
                                                    parse_mode: None,
                                                };
                                                send_with_retry(&*sender, reply).await;
                                            }
                                        }
                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        if let Ok(mut pq) = pending_for_worker.lock() {
                                            pq.remove(&worker_chat_id);
                                        }
                                        return;
                                    }

                                    // Pending raw key paste (from /addkey unsafe)
                                    if pending_raw_keys_worker.lock().await.remove(&msg.chat_id) {
                                        // Treat the message as a raw API key — falls through
                                        // to the normal detect_api_key path below
                                    }

                                    // Check if agent is available
                                    let agent = {
                                        let guard = agent_state.read().await;
                                        guard.as_ref().cloned()
                                    };

                                    if let Some(agent) = agent {
                                        // ── Detect new API key mid-conversation ────
                                        let msg_text_peek = msg.text.as_deref().unwrap_or("");
                                        if let Some(cred) = detect_api_key(msg_text_peek) {
                                            let model = default_model(cred.provider).to_string();
                                            let effective_base_url = cred.base_url.clone().or_else(|| base_url.clone());

                                            // Validate the key BEFORE saving — don't brick the agent
                                            let test_config = skyclaw_core::types::config::ProviderConfig {
                                                name: Some(cred.provider.to_string()),
                                                api_key: Some(cred.api_key.clone()),
                                                keys: vec![cred.api_key.clone()],
                                                model: Some(model.clone()),
                                                base_url: effective_base_url,
                                                extra_headers: std::collections::HashMap::new(),
                                            };

                                            match validate_provider_key(&test_config).await {
                                                Ok(_validated_provider) => {
                                                    // Key is valid — now save and reload with all keys
                                                    if let Err(e) = save_credentials(cred.provider, &cred.api_key, &model, cred.base_url.as_deref()).await {
                                                        tracing::error!(error = %e, "Failed to save new key");
                                                    } else if let Some((name, keys, mdl, saved_base_url)) = load_active_provider_keys() {
                                                        let reload_base_url = saved_base_url.or_else(|| base_url.clone());
                                                        let reload_config = skyclaw_core::types::config::ProviderConfig {
                                                            name: Some(name.clone()),
                                                            api_key: keys.first().cloned(),
                                                            keys: keys.clone(),
                                                            model: Some(mdl.clone()),
                                                            base_url: reload_base_url,
                                                            extra_headers: std::collections::HashMap::new(),
                                                        };
                                                        if let Ok(new_provider) = skyclaw_providers::create_provider(&reload_config) {
                                                            let new_agent = Arc::new(skyclaw_agent::AgentRuntime::with_limits(
                                                                Arc::from(new_provider),
                                                                memory.clone(),
                                                                tools_template.clone(),
                                                                mdl.clone(),
                                                                Some(build_system_prompt()),
                                                                max_turns,
                                                                max_ctx,
                                                                max_rounds,
                                                                max_task_duration,
                                                                max_spend,
                                                            ).with_v2_optimizations(v2_opt));
                                                            *agent_state.write().await = Some(new_agent);
                                                            let key_count = keys.len();
                                                            let reply = skyclaw_core::types::message::OutboundMessage {
                                                                chat_id: msg.chat_id.clone(),
                                                                text: format!(
                                                                    "Key verified and added for {}! Now using {} key{} with model {}.",
                                                                    name, key_count,
                                                                    if key_count > 1 { "s (rotation on error)" } else { "" },
                                                                    mdl
                                                                ),
                                                                reply_to: Some(msg.id.clone()),
                                                                parse_mode: None,
                                                            };
                                                            send_with_retry(&*sender, reply).await;
                                                            tracing::info!(
                                                                provider = %name,
                                                                key_count = key_count,
                                                                "Mid-conversation key validated and added — agent reloaded"
                                                            );
                                                        }
                                                    }
                                                }
                                                Err(err) => {
                                                    // Key is invalid — DO NOT save, DO NOT switch
                                                    let reply = skyclaw_core::types::message::OutboundMessage {
                                                        chat_id: msg.chat_id.clone(),
                                                        text: format!(
                                                            "Invalid API key — {} returned an error:\n{}\n\nThe current provider is still active. Check the key and try again.",
                                                            cred.provider, err
                                                        ),
                                                        reply_to: Some(msg.id.clone()),
                                                        parse_mode: None,
                                                    };
                                                    send_with_retry(&*sender, reply).await;
                                                    tracing::warn!(
                                                        provider = %cred.provider,
                                                        error = %err,
                                                        "Mid-conversation key rejected — validation failed"
                                                    );
                                                }
                                            }

                                            // Skip processing the key message as a normal prompt
                                            is_heartbeat_clone.store(false, Ordering::Relaxed);
                                            interrupt_clone.store(false, Ordering::Relaxed);
                                            if let Ok(mut pq) = pending_for_worker.lock() {
                                                pq.remove(&worker_chat_id);
                                            }
                                            return;
                                        }

                                        // ── Normal mode: process with agent ────

                                        // Download attachments
                                        if !msg.attachments.is_empty() {
                                            if let Some(ft) = sender.file_transfer() {
                                                match ft.receive_file(&msg).await {
                                                    Ok(files) => {
                                                        let mut file_notes = Vec::new();
                                                        for file in &files {
                                                            let save_path = workspace_path.join(&file.name);
                                                            if let Err(e) = tokio::fs::write(&save_path, &file.data).await {
                                                                tracing::error!(error = %e, file = %file.name, "Failed to save attachment");
                                                            } else {
                                                                tracing::info!(file = %file.name, size = file.size, "Saved attachment to workspace");
                                                                file_notes.push(format!(
                                                                    "[File received: {} ({}, {} bytes) — saved to workspace/{}]",
                                                                    file.name, file.mime_type, file.size, file.name
                                                                ));
                                                            }
                                                        }
                                                        if !file_notes.is_empty() {
                                                            let prefix = file_notes.join("\n");
                                                            let existing = msg.text.take().unwrap_or_default();
                                                            msg.text = Some(format!("{}\n{}", prefix, existing));
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::error!(error = %e, "Failed to download attachments");
                                                    }
                                                }
                                            }
                                        }

                                        let mut session = skyclaw_core::types::session::SessionContext {
                                            session_id: format!("{}-{}", msg.channel, msg.chat_id),
                                            user_id: msg.user_id.clone(),
                                            channel: msg.channel.clone(),
                                            chat_id: msg.chat_id.clone(),
                                            history: persistent_history.clone(),
                                            workspace_path: workspace_path.clone(),
                                        };

                                        // ── Panic-guarded message processing ─────────
                                        // Wraps process_message in catch_unwind so a panic
                                        // in context building, tool execution, or provider
                                        // parsing doesn't kill the per-chat worker loop.
                                        // The worker survives and continues processing the
                                        // next message — the user gets an error reply
                                        // instead of permanent silence.
                                        let process_result = AssertUnwindSafe(
                                            agent.process_message(&msg, &mut session, interrupt_flag, Some(pending_for_worker.clone()))
                                        )
                                        .catch_unwind()
                                        .await;

                                        match process_result {
                                            Ok(Ok((mut reply, turn_usage))) => {
                                                reply.text = censor_secrets(&reply.text);
                                                send_with_retry(&*sender, reply).await;

                                                // Record usage
                                                let record = skyclaw_core::UsageRecord {
                                                    id: uuid::Uuid::new_v4().to_string(),
                                                    chat_id: msg.chat_id.clone(),
                                                    session_id: format!("{}-{}", msg.channel, msg.chat_id),
                                                    timestamp: chrono::Utc::now(),
                                                    api_calls: turn_usage.api_calls,
                                                    input_tokens: turn_usage.input_tokens,
                                                    output_tokens: turn_usage.output_tokens,
                                                    tools_used: turn_usage.tools_used,
                                                    total_cost_usd: turn_usage.total_cost_usd,
                                                    provider: turn_usage.provider.clone(),
                                                    model: turn_usage.model.clone(),
                                                };
                                                if let Err(e) = usage_store_worker.record_usage(record).await {
                                                    tracing::error!(error = %e, "Failed to record usage");
                                                }

                                                // Display usage summary if enabled
                                                if turn_usage.api_calls > 0 {
                                                    if let Ok(enabled) = usage_store_worker.is_usage_display_enabled(&msg.chat_id).await {
                                                        if enabled {
                                                            let usage_msg = skyclaw_core::types::message::OutboundMessage {
                                                                chat_id: msg.chat_id.clone(),
                                                                text: turn_usage.format_summary(),
                                                                reply_to: None,
                                                                parse_mode: None,
                                                            };
                                                            send_with_retry(&*sender, usage_msg).await;
                                                        }
                                                    }
                                                }
                                            }
                                            Ok(Err(e)) => {
                                                tracing::error!(error = %e, "Agent processing error");
                                                let error_reply = skyclaw_core::types::message::OutboundMessage {
                                                    chat_id: msg.chat_id.clone(),
                                                    text: censor_secrets(&format!("Error: {}", e)),
                                                    reply_to: Some(msg.id.clone()),
                                                    parse_mode: None,
                                                };
                                                send_with_retry(&*sender, error_reply).await;
                                            }
                                            Err(panic_info) => {
                                                // ── Panic recovered — worker stays alive ────
                                                let panic_msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                                                    s.clone()
                                                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                                                    s.to_string()
                                                } else {
                                                    "internal error".to_string()
                                                };
                                                tracing::error!(
                                                    chat_id = %msg.chat_id,
                                                    panic = %panic_msg,
                                                    "PANIC RECOVERED in message processing — worker continues"
                                                );
                                                let error_reply = skyclaw_core::types::message::OutboundMessage {
                                                    chat_id: msg.chat_id.clone(),
                                                    text: "An internal error occurred while processing your message. I've recovered and am ready for your next message.".to_string(),
                                                    reply_to: Some(msg.id.clone()),
                                                    parse_mode: None,
                                                };
                                                send_with_retry(&*sender, error_reply).await;
                                                // Session history may be corrupted after a panic.
                                                // Trim the last entry if it was partially added.
                                                if persistent_history.len() < session.history.len() {
                                                    // Panic happened after adding user msg but before
                                                    // assistant reply — rollback to pre-message state.
                                                    session.history = persistent_history.clone();
                                                }
                                            }
                                        }

                                        // ── Persist session history for next message ────
                                        // Cap to last 200 messages to prevent unbounded memory growth
                                        persistent_history = session.history;
                                        if persistent_history.len() > 200 {
                                            let drain_count = persistent_history.len() - 200;
                                            persistent_history.drain(..drain_count);
                                        }

                                        // ── Save conversation history to memory backend ──
                                        if let Ok(json) = serde_json::to_string(&persistent_history) {
                                            let entry = skyclaw_core::MemoryEntry {
                                                id: history_key.clone(),
                                                content: json,
                                                metadata: serde_json::json!({"chat_id": worker_chat_id}),
                                                timestamp: chrono::Utc::now(),
                                                session_id: Some(worker_chat_id.clone()),
                                                entry_type: skyclaw_core::MemoryEntryType::Conversation,
                                            };
                                            if let Err(e) = memory.store(entry).await {
                                                tracing::warn!(
                                                    chat_id = %worker_chat_id,
                                                    error = %e,
                                                    "Failed to persist conversation history"
                                                );
                                            }
                                        }

                                        // ── Hot-reload: check if credentials changed ────
                                        if let Some((new_name, new_keys, new_model, saved_base_url)) = load_active_provider_keys() {
                                            let current_model = agent.model().to_string();
                                            if new_model != current_model || new_keys.len() > 1 {
                                                // Filter out placeholder keys before reloading
                                                let valid_keys: Vec<String> = new_keys.into_iter()
                                                    .filter(|k| !is_placeholder_key(k))
                                                    .collect();
                                                if valid_keys.is_empty() {
                                                    tracing::warn!(
                                                        provider = %new_name,
                                                        "Hot-reload skipped — all keys are placeholders"
                                                    );
                                                } else {
                                                    tracing::info!(
                                                        old_model = %current_model,
                                                        new_model = %new_model,
                                                        key_count = valid_keys.len(),
                                                        "Credentials changed — validating before hot-reload"
                                                    );
                                                    let effective_base_url = saved_base_url.or_else(|| base_url.clone());
                                                    let reload_config = skyclaw_core::types::config::ProviderConfig {
                                                        name: Some(new_name.clone()),
                                                        api_key: valid_keys.first().cloned(),
                                                        keys: valid_keys,
                                                        model: Some(new_model.clone()),
                                                        base_url: effective_base_url,
                                                        extra_headers: std::collections::HashMap::new(),
                                                    };
                                                    match validate_provider_key(&reload_config).await {
                                                        Ok(validated_provider) => {
                                                            let new_agent = Arc::new(skyclaw_agent::AgentRuntime::with_limits(
                                                                validated_provider,
                                                                memory.clone(),
                                                                tools_template.clone(),
                                                                new_model.clone(),
                                                                Some(build_system_prompt()),
                                                                max_turns,
                                                                max_ctx,
                                                                max_rounds,
                                                                max_task_duration,
                                                                max_spend,
                                                            ).with_v2_optimizations(v2_opt));
                                                            *agent_state.write().await = Some(new_agent);
                                                            tracing::info!(provider = %new_name, model = %new_model, "Agent hot-reloaded (key validated)");
                                                        }
                                                        Err(err) => {
                                                            tracing::warn!(
                                                                provider = %new_name,
                                                                error = %err,
                                                                "Hot-reload aborted — validation failed, reverting model"
                                                            );
                                                            // Revert credentials.toml to the working model
                                                            if let Some(mut creds) = load_credentials_file() {
                                                                for p in &mut creds.providers {
                                                                    if p.name == new_name {
                                                                        p.model = current_model.clone();
                                                                    }
                                                                }
                                                                if let Ok(content) = toml::to_string_pretty(&creds) {
                                                                    let _ = std::fs::write(credentials_path(), &content);
                                                                    tracing::info!(
                                                                        model = %current_model,
                                                                        "Reverted credentials.toml to working model"
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        // ── Onboarding / add-key mode: detect API key ────
                                        let msg_text = msg.text.as_deref().unwrap_or("");

                                        if let Some(cred) = detect_api_key(msg_text) {
                                            let provider_name = cred.provider;
                                            let api_key = cred.api_key;
                                            let custom_base_url = cred.base_url;
                                            let model = default_model(provider_name).to_string();
                                            // Load existing keys for this provider (if any)
                                            let mut all_keys = vec![api_key.clone()];
                                            if let Some(creds) = load_credentials_file() {
                                                if let Some(existing) = creds.providers.iter().find(|p| p.name == provider_name) {
                                                    for k in &existing.keys {
                                                        if !all_keys.contains(k) {
                                                            all_keys.push(k.clone());
                                                        }
                                                    }
                                                }
                                            }
                                            let effective_base_url = custom_base_url.clone().or_else(|| base_url.clone());
                                            let provider_config = skyclaw_core::types::config::ProviderConfig {
                                                name: Some(provider_name.to_string()),
                                                api_key: Some(api_key.clone()),
                                                keys: all_keys,
                                                model: Some(model.clone()),
                                                base_url: effective_base_url,
                                                extra_headers: std::collections::HashMap::new(),
                                            };

                                            match skyclaw_providers::create_provider(&provider_config) {
                                                Ok(_provider) => {
                                                    // Use shared validation (handles auth vs non-auth errors)
                                                    match validate_provider_key(&provider_config).await {
                                                        Ok(validated_provider) => {
                                                            // Key is valid — create agent and go online
                                                            let new_agent = Arc::new(skyclaw_agent::AgentRuntime::with_limits(
                                                                validated_provider,
                                                                memory.clone(),
                                                                tools_template.clone(),
                                                                model.clone(),
                                                                Some(build_system_prompt()),
                                                                max_turns,
                                                                max_ctx,
                                                                max_rounds,
                                                                max_task_duration,
                                                                max_spend,
                                                            ).with_v2_optimizations(v2_opt));
                                                            *agent_state.write().await = Some(new_agent);

                                                            if let Err(e) = save_credentials(provider_name, &api_key, &model, custom_base_url.as_deref()).await {
                                                                tracing::error!(error = %e, "Failed to save credentials");
                                                            }

                                                            let proxy_note = if custom_base_url.is_some() {
                                                                " (via proxy)"
                                                            } else {
                                                                ""
                                                            };
                                                            let reply = skyclaw_core::types::message::OutboundMessage {
                                                                chat_id: msg.chat_id.clone(),
                                                                text: format!(
                                                                    "API key verified! Configured {}{} with model {}.\n\nSkyClaw is online! You can:\n- Add more keys anytime (just paste them)\n- Use a proxy: \"proxy openai https://your-proxy/v1 your-key\"\n- Change settings in natural language\n\nHow can I help?",
                                                                    provider_name, proxy_note, model
                                                                ),
                                                                reply_to: Some(msg.id.clone()),
                                                                parse_mode: None,
                                                            };
                                                            send_with_retry(&*sender, reply).await;
                                                            tracing::info!(provider = %provider_name, model = %model, "API key validated — agent online");
                                                        }
                                                        Err(e) => {
                                                            // Key failed auth validation
                                                            let reply = skyclaw_core::types::message::OutboundMessage {
                                                                chat_id: msg.chat_id.clone(),
                                                                text: format!(
                                                                    "Invalid API key — the {} API returned an error:\n{}\n\nPlease check your key and paste it again.",
                                                                    provider_name, e
                                                                ),
                                                                reply_to: Some(msg.id.clone()),
                                                                parse_mode: None,
                                                            };
                                                            send_with_retry(&*sender, reply).await;
                                                            tracing::warn!(provider = %provider_name, error = %e, "API key validation failed");
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    let reply = skyclaw_core::types::message::OutboundMessage {
                                                        chat_id: msg.chat_id.clone(),
                                                        text: format!("Failed to configure provider: {}", e),
                                                        reply_to: Some(msg.id.clone()),
                                                        parse_mode: None,
                                                    };
                                                    send_with_retry(&*sender, reply).await;
                                                }
                                            }
                                        } else {
                                            // Auto-generate OTK and send onboarding with setup link
                                            let otk = setup_tokens_worker.generate(&msg.chat_id).await;
                                            let otk_hex = hex::encode(otk);
                                            let link = format!(
                                                "https://nagisanzenin.github.io/skyclaw/setup#{}",
                                                otk_hex
                                            );
                                            let reply = skyclaw_core::types::message::OutboundMessage {
                                                chat_id: msg.chat_id.clone(),
                                                text: onboarding_message_with_link(&link),
                                                reply_to: Some(msg.id.clone()),
                                                parse_mode: None,
                                            };
                                            send_with_retry(&*sender, reply).await;

                                            // Send format reference as separate message for easy copy-paste
                                            let ref_msg = skyclaw_core::types::message::OutboundMessage {
                                                chat_id: msg.chat_id.clone(),
                                                text: ONBOARDING_REFERENCE.to_string(),
                                                reply_to: None,
                                                parse_mode: None,
                                            };
                                            send_with_retry(&*sender, ref_msg).await;
                                        }
                                    }

                                    // Clear active state and pending queue
                                    is_heartbeat_clone.store(false, Ordering::Relaxed);
                                    interrupt_clone.store(false, Ordering::Relaxed);
                                    if let Ok(mut pq) = pending_for_worker.lock() {
                                        pq.remove(&worker_chat_id);
                                    }
                                    }).catch_unwind().await;

                                    // ── Outer panic safety net ─────────────────
                                    // If ANYTHING in the loop body panicked
                                    // (command handling, key detection, session
                                    // setup, etc.), recover here so the worker
                                    // loop survives.  The inner catch_unwind on
                                    // process_message provides more specific
                                    // recovery with usage tracking; this outer
                                    // one is a last-resort safety net.
                                    if let Err(panic_info) = outer_catch_result {
                                        let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                                            s.to_string()
                                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                                            s.clone()
                                        } else {
                                            "unknown panic".to_string()
                                        };
                                        tracing::error!(
                                            panic = %panic_msg,
                                            chat_id = %panic_chat_id,
                                            "Worker panic caught in outer safety net — recovering"
                                        );
                                        // Best-effort notification to the user
                                        let error_reply = skyclaw_core::types::message::OutboundMessage {
                                            chat_id: panic_chat_id.clone(),
                                            text: "An internal error occurred. Please try again.".to_string(),
                                            reply_to: Some(panic_msg_id.clone()),
                                            parse_mode: None,
                                        };
                                        let _ = sender.send_message(error_reply).await;
                                        // Ensure cleanup in case the panic skipped it
                                        is_heartbeat_clone.store(false, Ordering::Relaxed);
                                        interrupt_clone.store(false, Ordering::Relaxed);
                                        if let Ok(mut pq) = pending_for_worker.lock() {
                                            pq.remove(&worker_chat_id);
                                        }
                                    }
                                }
                            });

                            ChatSlot { tx: chat_tx, interrupt, is_heartbeat }
                        });

                        // Send message into the chat's dedicated queue.
                        // Clone the sender to release the borrow on slots, so we
                        // can remove the dead slot if the send fails.
                        if !is_heartbeat_msg {
                            let tx = slot.tx.clone();
                            drop(slots); // release Mutex guard before await
                            let inbound_backup = inbound.clone();
                            if let Err(e) = tx.send(inbound).await {
                                tracing::error!(
                                    chat_id = %chat_id,
                                    error = %e,
                                    "Chat worker dead — removing slot and re-dispatching"
                                );
                                let mut slots = chat_slots.lock().await;
                                slots.remove(&chat_id);
                                drop(slots); // release lock before re-dispatch
                                // Re-send through the unified channel so the
                                // dispatcher loop creates a fresh worker for
                                // this chat_id — zero messages lost.
                                if let Err(e2) = msg_tx_redispatch.send(inbound_backup).await {
                                    tracing::error!(
                                        chat_id = %chat_id,
                                        error = %e2,
                                        "Failed to re-dispatch message after worker death"
                                    );
                                }
                            }
                        }
                    }
                }));
            }

            // ── Start gateway + block ──────────────────────────
            println!("SkyClaw gateway starting...");
            println!("  Mode: {}", cli.mode);

            if let Some(agent) = agent_state.read().await.as_ref().cloned() {
                let gate = skyclaw_gateway::SkyGate::new(channels, agent, config.gateway.clone());
                task_handles.push(tokio::spawn(async move {
                    if let Err(e) = gate.start().await {
                        tracing::error!(error = %e, "Gateway error");
                    }
                }));
                println!("  Status: Online");
                println!(
                    "  Gateway: http://{}:{}",
                    config.gateway.host, config.gateway.port
                );
                println!(
                    "  Health: http://{}:{}/health",
                    config.gateway.host, config.gateway.port
                );
            } else {
                println!("  Status: Onboarding — send your API key via Telegram");
            }

            // Block until Ctrl+C, then drain gracefully
            tokio::signal::ctrl_c().await?;
            println!("\nSkyClaw shutting down gracefully...");

            // Drop the inbound message sender so the dispatcher loop exits
            // when its receiver sees the channel closed.
            drop(msg_tx);

            // Wait for spawned tasks with a timeout
            let drain_timeout = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                futures::future::join_all(task_handles),
            );
            match drain_timeout.await {
                Ok(_) => println!("All tasks drained cleanly."),
                Err(_) => println!("Drain timeout — forcing exit."),
            }
        }
        Commands::Chat => {
            println!("SkyClaw interactive chat");
            println!("Type '/quit' or '/exit' to quit.\n");

            // ── Resolve API credentials ────────────────────────
            let credentials: Option<(String, String, String)> = {
                if let Some(ref key) = config.provider.api_key {
                    if !key.is_empty() && !key.starts_with("${") {
                        let name = config
                            .provider
                            .name
                            .clone()
                            .unwrap_or_else(|| "anthropic".to_string());
                        let model = config
                            .provider
                            .model
                            .clone()
                            .unwrap_or_else(|| default_model(&name).to_string());
                        Some((name, key.clone(), model))
                    } else {
                        load_saved_credentials()
                    }
                } else {
                    load_saved_credentials()
                }
            };

            // ── Memory backend ─────────────────────────────────
            let memory_url = config.memory.path.clone().unwrap_or_else(|| {
                let data_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".skyclaw");
                if let Err(e) = std::fs::create_dir_all(&data_dir) {
                    tracing::warn!(error = %e, path = %data_dir.display(), "Failed to create directory");
                }
                format!("sqlite:{}/memory.db?mode=rwc", data_dir.display())
            });
            let memory: Arc<dyn skyclaw_core::Memory> = Arc::from(
                skyclaw_memory::create_memory_backend(&config.memory.backend, &memory_url).await?,
            );

            // ── CLI channel ────────────────────────────────────
            let workspace = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".skyclaw")
                .join("workspace");
            if let Err(e) = std::fs::create_dir_all(&workspace) {
                tracing::warn!(error = %e, path = %workspace.display(), "Failed to create directory");
            }
            let mut cli_channel = skyclaw_channels::CliChannel::new(workspace.clone());
            let cli_rx = cli_channel.take_receiver();
            cli_channel.start().await?;
            let cli_arc: Arc<dyn skyclaw_core::Channel> = Arc::new(cli_channel);

            // ── OTK state ──────────────────────────────────────
            let setup_tokens = skyclaw_gateway::SetupTokenStore::new();

            // ── Usage store ──────────────────────────────────────
            let usage_store: Arc<dyn skyclaw_core::UsageStore> =
                Arc::new(skyclaw_memory::SqliteUsageStore::new(&memory_url).await?);

            // ── Tools ──────────────────────────────────────────
            let pending_messages: skyclaw_tools::PendingMessages =
                Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            let censored_cli: Arc<dyn Channel> = Arc::new(SecretCensorChannel {
                inner: cli_arc.clone(),
            });
            let tools_template = skyclaw_tools::create_tools(
                &config.tools,
                Some(censored_cli),
                Some(pending_messages.clone()),
                Some(memory.clone()),
                Some(Arc::new(setup_tokens.clone()) as Arc<dyn skyclaw_core::SetupLinkGenerator>),
                Some(usage_store.clone()),
            );
            let base_url = config.provider.base_url.clone();

            // ── Build agent (if credentials available) ─────────
            let max_turns = config.agent.max_turns;
            let max_ctx = config.agent.max_context_tokens;
            let max_rounds = config.agent.max_tool_rounds;
            let max_task_duration = config.agent.max_task_duration_secs;
            let max_spend = config.agent.max_spend_usd;
            let v2_opt = config.agent.v2_optimizations;

            let mut agent_opt: Option<skyclaw_agent::AgentRuntime> = None;

            if let Some((pname, key, model)) = credentials {
                if !is_placeholder_key(&key) {
                    let (all_keys, saved_base_url) = load_active_provider_keys()
                        .map(|(_, keys, _, burl)| {
                            let valid: Vec<String> = keys
                                .into_iter()
                                .filter(|k| !is_placeholder_key(k))
                                .collect();
                            (valid, burl)
                        })
                        .unwrap_or_else(|| (vec![key.clone()], None));
                    let effective_base_url =
                        saved_base_url.or_else(|| config.provider.base_url.clone());
                    let provider_config = skyclaw_core::types::config::ProviderConfig {
                        name: Some(pname.clone()),
                        api_key: Some(key.clone()),
                        keys: all_keys,
                        model: Some(model.clone()),
                        base_url: effective_base_url,
                        extra_headers: config.provider.extra_headers.clone(),
                    };
                    match skyclaw_providers::create_provider(&provider_config) {
                        Ok(provider) => {
                            let provider: Arc<dyn skyclaw_core::Provider> = Arc::from(provider);
                            let system_prompt = Some(build_system_prompt());
                            agent_opt = Some(
                                skyclaw_agent::AgentRuntime::with_limits(
                                    provider,
                                    memory.clone(),
                                    tools_template.clone(),
                                    model.clone(),
                                    system_prompt,
                                    max_turns,
                                    max_ctx,
                                    max_rounds,
                                    max_task_duration,
                                    max_spend,
                                )
                                .with_v2_optimizations(v2_opt),
                            );
                            println!("Connected to {} (model: {})", pname, model);
                            if max_spend > 0.0 {
                                println!("Budget: ${:.2} per session", max_spend);
                            } else {
                                println!("Budget: unlimited");
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to create provider: {}", e);
                        }
                    }
                }
            }

            if agent_opt.is_none() {
                println!("No API key configured — running in onboarding mode.");
                // Auto-generate OTK and show setup link immediately
                let otk = setup_tokens.generate("cli").await;
                let otk_hex = hex::encode(otk);
                let link = format!("https://nagisanzenin.github.io/skyclaw/setup#{}", otk_hex);
                println!("\n{}", onboarding_message_with_link(&link));
                println!("\n{}", ONBOARDING_REFERENCE);
            }
            println!("---\n");

            // ── Message loop ───────────────────────────────────
            let Some(mut rx) = cli_rx else {
                eprintln!("CLI channel receiver unavailable");
                return Ok(());
            };
            // ── Restore CLI conversation history from memory backend ──
            let cli_history_key = "chat_history:cli".to_string();
            let mut history: Vec<skyclaw_core::types::message::ChatMessage> =
                match memory.get(&cli_history_key).await {
                    Ok(Some(entry)) => match serde_json::from_str(&entry.content) {
                        Ok(h) => {
                            let count = Vec::<skyclaw_core::types::message::ChatMessage>::len(&h);
                            if count > 0 {
                                println!("  Restored {} messages from previous session.", count);
                            }
                            h
                        }
                        Err(_) => Vec::new(),
                    },
                    _ => Vec::new(),
                };

            while let Some(msg) = rx.recv().await {
                let msg_text = msg.text.as_deref().unwrap_or("");
                let cmd_lower = msg_text.trim().to_lowercase();

                // ── Command interception (same as gateway) ─────
                // /addkey — secure OTK flow
                if cmd_lower == "/addkey" {
                    let otk = setup_tokens.generate(&msg.chat_id).await;
                    let otk_hex = hex::encode(otk);
                    let link = format!("https://nagisanzenin.github.io/skyclaw/setup#{}", otk_hex);
                    println!(
                        "\nSecure key setup:\n\n\
                         1. Open this link:\n{}\n\n\
                         2. Paste your API key in the form\n\
                         3. Copy the encrypted blob\n\
                         4. Paste it back here\n\n\
                         Link expires in 10 minutes.\n\n\
                         For a quick (less secure) method: /addkey unsafe\n",
                        link
                    );
                    eprint!("skyclaw> ");
                    continue;
                }

                // /addkey unsafe
                if cmd_lower == "/addkey unsafe" {
                    println!("\nPaste your API key below.");
                    println!("Warning: the key will be visible in terminal history.");
                    println!("For a secure method, use /addkey instead.\n");
                    eprint!("skyclaw> ");
                    continue;
                }

                // /keys
                if cmd_lower == "/keys" {
                    println!("\n{}\n", list_configured_providers());
                    eprint!("skyclaw> ");
                    continue;
                }

                // /removekey <provider>
                if cmd_lower.starts_with("/removekey") {
                    let provider_arg = msg_text.trim()["/removekey".len()..].trim();
                    println!("\n{}\n", remove_provider(provider_arg));
                    if !provider_arg.is_empty() && load_active_provider_keys().is_none() {
                        agent_opt = None;
                        println!("All providers removed — agent offline.\n");
                    }
                    eprint!("skyclaw> ");
                    continue;
                }

                // /usage — show usage summary
                if cmd_lower == "/usage" {
                    match usage_store.usage_summary(&msg.chat_id).await {
                        Ok(summary) => {
                            if summary.turn_count == 0 {
                                println!("\nNo usage records for this chat yet.\n");
                            } else {
                                println!(
                                    "\nUsage Summary\nTurns: {}\nAPI Calls: {}\nInput Tokens: {}\nOutput Tokens: {}\nCombined Tokens: {}\nTools Used: {}\nTotal Cost: ${:.4}\n",
                                    summary.turn_count,
                                    summary.total_api_calls,
                                    summary.total_input_tokens,
                                    summary.total_output_tokens,
                                    summary.combined_tokens(),
                                    summary.total_tools_used,
                                    summary.total_cost_usd,
                                );
                            }
                        }
                        Err(e) => eprintln!("Failed to query usage: {}", e),
                    }
                    eprint!("skyclaw> ");
                    continue;
                }

                // enc:v1: — encrypted blob from OTK flow
                if msg_text.trim().starts_with("enc:v1:") {
                    let blob_b64 = &msg_text.trim()["enc:v1:".len()..];
                    match decrypt_otk_blob(blob_b64, &setup_tokens, &msg.chat_id).await {
                        Ok(api_key_text) => {
                            if let Some(cred) = detect_api_key(&api_key_text) {
                                let model = default_model(cred.provider).to_string();
                                let effective_base_url =
                                    cred.base_url.clone().or_else(|| base_url.clone());
                                let test_config = skyclaw_core::types::config::ProviderConfig {
                                    name: Some(cred.provider.to_string()),
                                    api_key: Some(cred.api_key.clone()),
                                    keys: vec![cred.api_key.clone()],
                                    model: Some(model.clone()),
                                    base_url: effective_base_url,
                                    extra_headers: std::collections::HashMap::new(),
                                };
                                match validate_provider_key(&test_config).await {
                                    Ok(validated_provider) => {
                                        if let Err(e) = save_credentials(
                                            cred.provider,
                                            &cred.api_key,
                                            &model,
                                            cred.base_url.as_deref(),
                                        )
                                        .await
                                        {
                                            eprintln!("Failed to save credentials: {}", e);
                                        }
                                        let system_prompt = Some(build_system_prompt());
                                        agent_opt = Some(
                                            skyclaw_agent::AgentRuntime::with_limits(
                                                validated_provider,
                                                memory.clone(),
                                                tools_template.clone(),
                                                model.clone(),
                                                system_prompt,
                                                max_turns,
                                                max_ctx,
                                                max_rounds,
                                                max_task_duration,
                                                max_spend,
                                            )
                                            .with_v2_optimizations(v2_opt),
                                        );
                                        println!(
                                            "\nAPI key securely received and verified! Configured {} with model {}.",
                                            cred.provider, model
                                        );
                                        println!("SkyClaw is online.\n");
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "\nKey decrypted but validation failed — {} returned:\n{}\nCheck the key and try /addkey again.\n",
                                            cred.provider, err
                                        );
                                    }
                                }
                            } else {
                                eprintln!(
                                    "\nDecrypted successfully but couldn't detect the provider.\nMake sure you pasted a valid API key in the setup page.\n"
                                );
                            }
                        }
                        Err(err) => {
                            eprintln!("\n{}\n", err);
                        }
                    }
                    eprint!("skyclaw> ");
                    continue;
                }

                // Detect raw API key paste
                if let Some(cred) = detect_api_key(msg_text) {
                    let model = default_model(cred.provider).to_string();
                    let effective_base_url = cred.base_url.clone().or_else(|| base_url.clone());
                    let test_config = skyclaw_core::types::config::ProviderConfig {
                        name: Some(cred.provider.to_string()),
                        api_key: Some(cred.api_key.clone()),
                        keys: vec![cred.api_key.clone()],
                        model: Some(model.clone()),
                        base_url: effective_base_url,
                        extra_headers: std::collections::HashMap::new(),
                    };
                    match validate_provider_key(&test_config).await {
                        Ok(validated_provider) => {
                            if let Err(e) = save_credentials(
                                cred.provider,
                                &cred.api_key,
                                &model,
                                cred.base_url.as_deref(),
                            )
                            .await
                            {
                                eprintln!("Failed to save credentials: {}", e);
                            }
                            let system_prompt = Some(build_system_prompt());
                            agent_opt = Some(
                                skyclaw_agent::AgentRuntime::with_limits(
                                    validated_provider,
                                    memory.clone(),
                                    tools_template.clone(),
                                    model.clone(),
                                    system_prompt,
                                    max_turns,
                                    max_ctx,
                                    max_rounds,
                                    max_task_duration,
                                    max_spend,
                                )
                                .with_v2_optimizations(v2_opt),
                            );
                            println!(
                                "\nAPI key verified! Configured {} with model {}.",
                                cred.provider, model
                            );
                            println!("SkyClaw is online.\n");
                        }
                        Err(err) => {
                            eprintln!(
                                "\nInvalid API key — {} returned:\n{}\nCheck the key and try again.\n",
                                cred.provider, err
                            );
                        }
                    }
                    eprint!("skyclaw> ");
                    continue;
                }

                // ── Normal agent processing ────────────────────
                if let Some(ref agent) = agent_opt {
                    let mut session = skyclaw_core::types::session::SessionContext {
                        session_id: "cli-cli".to_string(),
                        user_id: msg.user_id.clone(),
                        channel: msg.channel.clone(),
                        chat_id: msg.chat_id.clone(),
                        history: history.clone(),
                        workspace_path: workspace.clone(),
                    };

                    let process_result =
                        AssertUnwindSafe(agent.process_message(&msg, &mut session, None, None))
                            .catch_unwind()
                            .await;

                    match process_result {
                        Ok(Ok((mut reply, turn_usage))) => {
                            reply.text = censor_secrets(&reply.text);
                            cli_arc.send_message(reply).await.ok();

                            // Record usage
                            let record = skyclaw_core::UsageRecord {
                                id: uuid::Uuid::new_v4().to_string(),
                                chat_id: msg.chat_id.clone(),
                                session_id: "cli-cli".to_string(),
                                timestamp: chrono::Utc::now(),
                                api_calls: turn_usage.api_calls,
                                input_tokens: turn_usage.input_tokens,
                                output_tokens: turn_usage.output_tokens,
                                tools_used: turn_usage.tools_used,
                                total_cost_usd: turn_usage.total_cost_usd,
                                provider: turn_usage.provider.clone(),
                                model: turn_usage.model.clone(),
                            };
                            if let Err(e) = usage_store.record_usage(record).await {
                                tracing::error!(error = %e, "Failed to record usage");
                            }

                            // Display usage summary if enabled
                            if turn_usage.api_calls > 0 {
                                if let Ok(enabled) =
                                    usage_store.is_usage_display_enabled(&msg.chat_id).await
                                {
                                    if enabled {
                                        println!("\n{}", turn_usage.format_summary());
                                    }
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            eprintln!("  [error: {}]", e);
                            eprint!("skyclaw> ");
                        }
                        Err(panic_info) => {
                            let panic_msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                                s.to_string()
                            } else {
                                "internal error".to_string()
                            };
                            eprintln!("  [panic recovered: {}]", panic_msg);
                            tracing::error!(panic = %panic_msg, "PANIC RECOVERED in CLI processing");
                            // Rollback session to pre-message state
                            session.history = history.clone();
                        }
                    }

                    history = session.history;

                    // ── Save CLI conversation history to memory backend ──
                    if let Ok(json) = serde_json::to_string(&history) {
                        let entry = skyclaw_core::MemoryEntry {
                            id: cli_history_key.clone(),
                            content: json,
                            metadata: serde_json::json!({"chat_id": "cli"}),
                            timestamp: chrono::Utc::now(),
                            session_id: Some("cli".to_string()),
                            entry_type: skyclaw_core::MemoryEntryType::Conversation,
                        };
                        if let Err(e) = memory.store(entry).await {
                            tracing::warn!(error = %e, "Failed to persist CLI conversation history");
                        }
                    }
                } else {
                    // Auto-generate fresh OTK for onboarding
                    let otk = setup_tokens.generate("cli").await;
                    let otk_hex = hex::encode(otk);
                    let link = format!("https://nagisanzenin.github.io/skyclaw/setup#{}", otk_hex);
                    println!("\n{}", onboarding_message_with_link(&link));
                    println!("\n{}\n", ONBOARDING_REFERENCE);
                    eprint!("skyclaw> ");
                }
            }

            println!("\nSkyClaw chat ended.");
        }
        Commands::Status => {
            println!("SkyClaw Status");
            println!("  Mode: {}", config.skyclaw.mode);
            println!("  Gateway: {}:{}", config.gateway.host, config.gateway.port);
            println!(
                "  Provider: {}",
                config.provider.name.as_deref().unwrap_or("not configured")
            );
            println!("  Memory: {}", config.memory.backend);
            println!("  Vault: {}", config.vault.backend);
        }
        Commands::Skill { command } => match command {
            SkillCommands::List => {
                println!("Installed skills:");
            }
            SkillCommands::Info { name } => {
                println!("Skill info: {}", name);
            }
            SkillCommands::Install { path } => {
                println!("Installing skill from: {}", path);
            }
        },
        Commands::Config { command } => match command {
            ConfigCommands::Validate => {
                println!("Configuration valid.");
                println!("  Gateway: {}:{}", config.gateway.host, config.gateway.port);
                println!(
                    "  Provider: {}",
                    config.provider.name.as_deref().unwrap_or("none")
                );
                println!("  Memory backend: {}", config.memory.backend);
                println!("  Channels: {}", config.channel.len());
            }
            ConfigCommands::Show => {
                let output = toml::to_string_pretty(&config)?;
                println!("{}", output);
            }
        },
        Commands::Version => {
            println!("skyclaw {}", env!("CARGO_PKG_VERSION"));
            println!("Cloud-native Rust AI agent runtime — Telegram-native");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_api_key: auto-detect from prefix ──────────────────────

    #[test]
    fn detect_anthropic_key() {
        let result = detect_api_key("sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(result.unwrap().provider, "anthropic");
    }

    #[test]
    fn detect_openai_key() {
        let result = detect_api_key("sk-proj-abcdefghijklmnopqrstuv");
        assert_eq!(result.unwrap().provider, "openai");
    }

    #[test]
    fn detect_openrouter_key() {
        let result = detect_api_key("sk-or-v1-abcdefghijklmnopqrstuv");
        assert_eq!(result.unwrap().provider, "openrouter");
    }

    #[test]
    fn detect_grok_key() {
        let result = detect_api_key("xai-abcdefghijklmnopqrstuvwxyz");
        assert_eq!(result.unwrap().provider, "grok");
    }

    #[test]
    fn detect_gemini_key() {
        let result = detect_api_key("AIzaSyA-abcdefghijklmnopqrstu");
        assert_eq!(result.unwrap().provider, "gemini");
    }

    #[test]
    fn detect_unknown_key_returns_none() {
        assert!(detect_api_key("unknown-key-format-here").is_none());
    }

    // ── detect_api_key: explicit provider:key format ─────────────────

    #[test]
    fn explicit_minimax_key() {
        let result = detect_api_key("minimax:eyJhbGciOiJSUzI1NiIsInR5cCI6").unwrap();
        assert_eq!(result.provider, "minimax");
        assert_eq!(result.api_key, "eyJhbGciOiJSUzI1NiIsInR5cCI6");
    }

    #[test]
    fn explicit_openrouter_key() {
        let result = detect_api_key("openrouter:sk-or-v1-abcdefghijklm").unwrap();
        assert_eq!(result.provider, "openrouter");
        assert_eq!(result.api_key, "sk-or-v1-abcdefghijklm");
    }

    #[test]
    fn explicit_grok_with_xai_alias() {
        let result = detect_api_key("xai:some-long-api-key-value").unwrap();
        assert_eq!(result.provider, "grok");
        assert_eq!(result.api_key, "some-long-api-key-value");
    }

    #[test]
    fn explicit_ollama_key() {
        let result = detect_api_key("ollama:some-long-ollama-api-key").unwrap();
        assert_eq!(result.provider, "ollama");
        assert_eq!(result.api_key, "some-long-ollama-api-key");
    }

    #[test]
    fn explicit_zai_key() {
        let result =
            detect_api_key("zai:24f7a8ebaa2f4cb1866b82b0670a5e6c.rPGt3alOjwddy4l1").unwrap();
        assert_eq!(result.provider, "zai");
        assert_eq!(
            result.api_key,
            "24f7a8ebaa2f4cb1866b82b0670a5e6c.rPGt3alOjwddy4l1"
        );
    }

    #[test]
    fn explicit_zhipu_key() {
        let result =
            detect_api_key("zhipu:24f7a8ebaa2f4cb1866b82b0670a5e6c.rPGt3alOjwddy4l1").unwrap();
        assert_eq!(result.provider, "zai");
    }

    #[test]
    fn explicit_format_case_insensitive() {
        let result = detect_api_key("MiniMax:eyJhbGciOiJSUzI1NiIsInR5cCI6");
        assert_eq!(result.unwrap().provider, "minimax");
    }

    #[test]
    fn explicit_format_short_key_rejected() {
        assert!(detect_api_key("minimax:short").is_none());
    }

    #[test]
    fn explicit_unknown_provider_falls_through() {
        assert!(detect_api_key("fakeprovider:some-key-value").is_none());
    }

    // ── detect_api_key: ordering (specific before generic) ───────────

    #[test]
    fn openrouter_not_misdetected_as_openai() {
        let result = detect_api_key("sk-or-v1-abcdefghijklmnopqrstuv");
        assert_eq!(result.unwrap().provider, "openrouter");
    }

    #[test]
    fn anthropic_not_misdetected_as_openai() {
        let result = detect_api_key("sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(result.unwrap().provider, "anthropic");
    }

    // ── detect_api_key: proxy format ────────────────────────────────

    #[test]
    fn proxy_with_key_value_format() {
        let result = detect_api_key(
            "proxy provider:openai base_url:https://my-proxy.com/v1 key:sk-test-key-12345678",
        )
        .unwrap();
        assert_eq!(result.provider, "openai");
        assert_eq!(result.api_key, "sk-test-key-12345678");
        assert_eq!(result.base_url.unwrap(), "https://my-proxy.com/v1");
    }

    #[test]
    fn proxy_with_positional_format() {
        let result =
            detect_api_key("proxy openai https://my-proxy.com/v1 sk-test-key-12345678").unwrap();
        assert_eq!(result.provider, "openai");
        assert_eq!(result.api_key, "sk-test-key-12345678");
        assert_eq!(result.base_url.unwrap(), "https://my-proxy.com/v1");
    }

    #[test]
    fn proxy_with_url_alias() {
        let result = detect_api_key(
            "proxy provider:anthropic url:https://claude-proxy.com/v1 key:sk-ant-test1234",
        )
        .unwrap();
        assert_eq!(result.provider, "anthropic");
        assert_eq!(result.base_url.unwrap(), "https://claude-proxy.com/v1");
    }

    #[test]
    fn proxy_defaults_to_openai() {
        let result = detect_api_key("proxy https://my-proxy.com/v1 sk-test-key-12345678").unwrap();
        assert_eq!(result.provider, "openai");
        assert_eq!(result.base_url.unwrap(), "https://my-proxy.com/v1");
    }

    #[test]
    fn proxy_too_few_tokens_returns_none() {
        assert!(detect_api_key("proxy openai").is_none());
    }

    // ── default_model ────────────────────────────────────────────────

    #[test]
    fn default_models_all_providers() {
        assert_eq!(default_model("anthropic"), "claude-sonnet-4-6");
        assert_eq!(default_model("openai"), "gpt-5.2");
        assert_eq!(default_model("gemini"), "gemini-3-flash-preview");
        assert_eq!(default_model("grok"), "grok-4-1-fast-non-reasoning");
        assert_eq!(default_model("xai"), "grok-4-1-fast-non-reasoning");
        assert_eq!(default_model("openrouter"), "anthropic/claude-sonnet-4-6");
        assert_eq!(default_model("minimax"), "MiniMax-M2.5");
        assert_eq!(default_model("zai"), "glm-4.7-flash");
        assert_eq!(default_model("ollama"), "llama3.3");
    }

    #[test]
    fn default_model_unknown_falls_back() {
        assert_eq!(default_model("unknown"), "claude-sonnet-4-6");
    }

    // ── is_placeholder_key ────────────────────────────────────────────

    #[test]
    fn placeholder_key_rejects_common_fakes() {
        assert!(is_placeholder_key("PASTE_YOUR_KEY_HERE"));
        assert!(is_placeholder_key("your_api_key"));
        assert!(is_placeholder_key("your-key-goes-here"));
        assert!(is_placeholder_key("insert_your_key_here"));
        assert!(is_placeholder_key("replace_with_your_key"));
        assert!(is_placeholder_key("placeholder_key_value"));
        assert!(is_placeholder_key("enter_your_api_key_here"));
        assert!(is_placeholder_key("xxxxxxxxxx"));
        assert!(is_placeholder_key("your_token_goes_here"));
    }

    #[test]
    fn placeholder_key_rejects_too_short() {
        assert!(is_placeholder_key("sk-abc"));
        assert!(is_placeholder_key("short"));
        assert!(is_placeholder_key(""));
    }

    #[test]
    fn placeholder_key_rejects_all_same_char() {
        assert!(is_placeholder_key("aaaaaaaaaa"));
        assert!(is_placeholder_key("0000000000"));
    }

    #[test]
    fn placeholder_key_accepts_real_keys() {
        assert!(!is_placeholder_key(
            "sk-ant-api03-abc123def456ghi789jkl012mno345pqr678stu"
        ));
        assert!(!is_placeholder_key("sk-proj-abcdefghijklmnopqrstuv"));
        assert!(!is_placeholder_key("sk-or-v1-abcdefghijklmnopqrstuv"));
        assert!(!is_placeholder_key("xai-abcdefghijklmnopqrstuvwxyz"));
        assert!(!is_placeholder_key("AIzaSyA-abcdefghijklmnopqrstu"));
        assert!(!is_placeholder_key("sk-test-key-12345678")); // valid test fixture format
    }

    #[test]
    fn detect_rejects_placeholder_auto_format() {
        // These match sk- prefix but are obvious placeholders
        assert!(detect_api_key("sk-your_key_here_12345").is_none());
        assert!(detect_api_key("sk-ant-paste_your_key_here").is_none());
    }

    // ── OTK: AES-256-GCM encrypt/decrypt round-trip ──────────────────

    #[tokio::test]
    async fn otk_encrypt_decrypt_round_trip() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Key, Nonce};

        let store = skyclaw_gateway::SetupTokenStore::new();
        let otk = store.generate("test-chat").await;

        // Simulate browser-side encryption
        let api_key = "sk-ant-api03-realkey1234567890abcdef";
        let key = Key::<Aes256Gcm>::from_slice(&otk);
        let cipher = Aes256Gcm::new(key);

        let mut iv = [0u8; 12];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut iv);
        let nonce = Nonce::from_slice(&iv);

        let ciphertext = cipher
            .encrypt(nonce, api_key.as_bytes())
            .expect("encryption failed");

        // Concatenate IV + ciphertext (matches WebCrypto format)
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&iv);
        blob.extend_from_slice(&ciphertext);

        let b64 = base64::engine::general_purpose::STANDARD.encode(&blob);

        // Decrypt using the OTK flow
        let result = decrypt_otk_blob(&b64, &store, "test-chat").await;
        assert_eq!(result.unwrap(), api_key);
    }

    #[tokio::test]
    async fn otk_decrypt_wrong_chat_id_fails() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Key, Nonce};

        let store = skyclaw_gateway::SetupTokenStore::new();
        let otk = store.generate("chat-a").await;

        let api_key = "sk-ant-api03-testkey123456789";
        let key = Key::<Aes256Gcm>::from_slice(&otk);
        let cipher = Aes256Gcm::new(key);
        let iv = [1u8; 12];
        let nonce = Nonce::from_slice(&iv);
        let ciphertext = cipher
            .encrypt(nonce, api_key.as_bytes())
            .expect("encryption failed");

        let mut blob = Vec::new();
        blob.extend_from_slice(&iv);
        blob.extend_from_slice(&ciphertext);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&blob);

        // Try to decrypt with wrong chat_id — should fail (no OTK)
        let result = decrypt_otk_blob(&b64, &store, "chat-b").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending setup link"));
    }

    #[tokio::test]
    async fn otk_decrypt_expired_token_fails() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Key, Nonce};

        let store = skyclaw_gateway::SetupTokenStore::with_ttl(std::time::Duration::from_millis(1));
        let otk = store.generate("chat-expire").await;

        let api_key = "sk-ant-api03-testkey123456789";
        let key = Key::<Aes256Gcm>::from_slice(&otk);
        let cipher = Aes256Gcm::new(key);
        let iv = [2u8; 12];
        let nonce = Nonce::from_slice(&iv);
        let ciphertext = cipher
            .encrypt(nonce, api_key.as_bytes())
            .expect("encryption failed");

        let mut blob = Vec::new();
        blob.extend_from_slice(&iv);
        blob.extend_from_slice(&ciphertext);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&blob);

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let result = decrypt_otk_blob(&b64, &store, "chat-expire").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No pending setup link"));
    }

    #[tokio::test]
    async fn otk_decrypt_tampered_blob_fails() {
        let store = skyclaw_gateway::SetupTokenStore::new();
        let _otk = store.generate("chat-tamper").await;

        // Tampered blob — valid base64 but wrong ciphertext
        let fake_blob = base64::engine::general_purpose::STANDARD.encode([0u8; 64]); // random bytes

        let result = decrypt_otk_blob(&fake_blob, &store, "chat-tamper").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Decryption failed"));
    }

    #[tokio::test]
    async fn otk_decrypt_invalid_base64_fails() {
        let store = skyclaw_gateway::SetupTokenStore::new();
        let _otk = store.generate("chat-b64").await;

        let result = decrypt_otk_blob("not!valid!base64!!!", &store, "chat-b64").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid base64"));
    }

    #[tokio::test]
    async fn otk_decrypt_too_short_blob_fails() {
        let store = skyclaw_gateway::SetupTokenStore::new();
        let _otk = store.generate("chat-short").await;

        let short_blob = base64::engine::general_purpose::STANDARD.encode([0u8; 10]);

        let result = decrypt_otk_blob(&short_blob, &store, "chat-short").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    #[test]
    fn enc_v1_prefix_detection() {
        assert!("enc:v1:SGVsbG8gV29ybGQ=".starts_with("enc:v1:"));
        assert!(!"sk-ant-api03-abc".starts_with("enc:v1:"));
        assert!(!"enc:v2:something".starts_with("enc:v1:"));
    }

    // ── Command parsing ──────────────────────────────────────────────

    #[test]
    fn command_addkey_detection() {
        assert_eq!("/addkey".trim().to_lowercase(), "/addkey");
        assert_eq!("/addkey ".trim().to_lowercase(), "/addkey");
        assert_eq!("  /addkey  ".trim().to_lowercase(), "/addkey");
    }

    #[test]
    fn command_addkey_unsafe_detection() {
        assert_eq!("/addkey unsafe".trim().to_lowercase(), "/addkey unsafe");
        assert_eq!("  /addkey unsafe  ".trim().to_lowercase(), "/addkey unsafe");
    }

    #[test]
    fn command_keys_detection() {
        assert_eq!("/keys".trim().to_lowercase(), "/keys");
    }

    #[test]
    fn command_removekey_detection() {
        let cmd = "/removekey openai";
        let lower = cmd.trim().to_lowercase();
        assert!(lower.starts_with("/removekey"));
        let provider = cmd.trim()["/removekey".len()..].trim();
        assert_eq!(provider, "openai");
    }

    #[test]
    fn command_removekey_no_provider() {
        let cmd = "/removekey";
        let provider = cmd.trim()["/removekey".len()..].trim();
        assert!(provider.is_empty());
    }

    // ── list/remove helpers ──────────────────────────────────────────

    #[test]
    fn list_providers_no_credentials() {
        // When no credentials file exists, returns helpful message
        let result = list_configured_providers();
        // Either returns provider list or "no providers" message — both valid
        assert!(!result.is_empty());
    }

    #[test]
    fn remove_provider_empty_name() {
        let result = remove_provider("");
        assert!(result.contains("Usage"));
    }

    // ── OTK hex encoding ─────────────────────────────────────────────

    #[test]
    fn otk_hex_encoding_format() {
        let bytes = [0xab_u8; 32];
        let hex_str = hex::encode(bytes);
        assert_eq!(hex_str.len(), 64); // 32 bytes = 64 hex chars
        assert!(hex_str.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Full end-to-end: generate OTK → hex encode (like /addkey) → decode hex
    /// (like browser) → encrypt (like browser) → format as enc:v1: → decrypt
    /// (like server). Verifies the entire chain is consistent.
    #[tokio::test]
    async fn otk_full_e2e_hex_roundtrip() {
        use aes_gcm::aead::{Aead, KeyInit};
        use aes_gcm::{Aes256Gcm, Key, Nonce};

        let store = skyclaw_gateway::SetupTokenStore::new();
        let otk = store.generate("e2e-chat").await;

        // Step 1: Server encodes OTK as hex (what goes into the URL fragment)
        let otk_hex = hex::encode(otk);
        assert_eq!(otk_hex.len(), 64);

        // Step 2: Browser decodes hex back to bytes (simulating JS hex decode)
        let browser_otk = hex::decode(&otk_hex).expect("hex decode failed");
        assert_eq!(browser_otk.len(), 32);
        assert_eq!(&browser_otk[..], &otk[..]);

        // Step 3: Browser encrypts with AES-256-GCM (simulating WebCrypto)
        let api_key = "sk-ant-api03-test1234567890abcdefghijklmnopqrs";
        let key = Key::<Aes256Gcm>::from_slice(&browser_otk);
        let cipher = Aes256Gcm::new(key);
        let mut iv = [0u8; 12];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut iv);
        let nonce = Nonce::from_slice(&iv);
        let ciphertext = cipher
            .encrypt(nonce, api_key.as_bytes())
            .expect("encryption failed");

        // Step 4: Browser builds "enc:v1:" blob
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&iv);
        blob.extend_from_slice(&ciphertext);
        let enc_blob = format!(
            "enc:v1:{}",
            base64::engine::general_purpose::STANDARD.encode(&blob)
        );

        // Step 5: Server detects prefix and decrypts
        assert!(enc_blob.starts_with("enc:v1:"));
        let blob_b64 = &enc_blob["enc:v1:".len()..];
        let result = decrypt_otk_blob(blob_b64, &store, "e2e-chat").await;
        assert_eq!(result.unwrap(), api_key);
    }

    /// Verify that detect_api_key works on decrypted OTK output for all providers.
    #[test]
    fn otk_decrypted_key_detection() {
        // These are the key formats users would paste into the setup page
        assert!(detect_api_key("sk-ant-api03-abcdefghijklmnop").is_some());
        assert!(detect_api_key("sk-proj-abcdefghijklmnop1234").is_some());
        assert!(detect_api_key("AIzaSyA-abcdefghijklmnopqrstu").is_some());
        assert!(detect_api_key("xai-abcdefghijklmnopqrstuvwxyz").is_some());
        assert!(detect_api_key("sk-or-v1-abcdefghijklmnopqrstu").is_some());
    }

    // ── censor_secrets: output filter ──────────────────────────────

    #[test]
    fn censor_no_credentials_file_returns_unchanged() {
        // When there are no credentials, text passes through unchanged
        let text = "Here is your key: sk-ant-test123456789";
        let result = censor_secrets(text);
        // Without credentials file, nothing to censor — returns as-is
        assert_eq!(result, text);
    }

    #[test]
    fn censor_replaces_known_key_in_text() {
        // Write a temporary credentials file for the test
        let path = credentials_path();
        let dir = path.parent().unwrap();
        std::fs::create_dir_all(dir).ok();

        // Save current file if exists, restore after test
        let backup = std::fs::read_to_string(&path).ok();

        let test_key = "sk-ant-test-SUPERSECRETKEY12345678";
        let creds_content = format!(
            "active = \"anthropic\"\n\n\
             [[providers]]\n\
             name = \"anthropic\"\n\
             keys = [\"{}\"]\n\
             model = \"claude-sonnet-4-6\"\n",
            test_key
        );
        std::fs::write(&path, &creds_content).expect("test: write credential file");

        let text = format!("Your API key is {} and it works great!", test_key);
        let censored = censor_secrets(&text);
        assert!(
            !censored.contains(test_key),
            "Key should be censored from output"
        );
        assert!(
            censored.contains("[REDACTED]"),
            "Should contain [REDACTED] placeholder"
        );
        assert_eq!(censored, "Your API key is [REDACTED] and it works great!");

        // Restore
        match backup {
            Some(content) => std::fs::write(&path, content).expect("test: restore credential file"),
            None => {
                std::fs::remove_file(&path).ok();
            }
        };
    }

    #[test]
    fn censor_ignores_placeholder_keys() {
        let text = "Your key is placeholder_or_empty";
        let result = censor_secrets(text);
        // Placeholder keys should not cause censoring
        assert_eq!(result, text);
    }
}
