use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::message::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, MessageContent, Role,
    StreamChunk, ToolDefinition, Usage,
};
use temm1e_core::Provider;
use tracing::{debug, error, info};

/// Anthropic Messages API provider with key rotation.
pub struct AnthropicProvider {
    client: Client,
    keys: Vec<String>,
    key_index: AtomicUsize,
    base_url: String,
    last_rotation: std::sync::Mutex<std::time::Instant>,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| Client::new()),
            keys: vec![api_key],
            key_index: AtomicUsize::new(0),
            base_url: "https://api.anthropic.com".to_string(),
            last_rotation: std::sync::Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(10),
            ),
        }
    }

    pub fn with_keys(mut self, keys: Vec<String>) -> Self {
        if !keys.is_empty() {
            self.keys = keys;
        }
        self
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    /// Get the current API key via round-robin rotation.
    fn current_key(&self) -> &str {
        if self.keys.is_empty() {
            return "";
        }
        let idx = self.key_index.load(Ordering::Relaxed) % self.keys.len();
        &self.keys[idx]
    }

    /// Advance to the next key (called on rate limit).
    /// Skips rotation if the last rotation was less than 2 seconds ago
    /// (all keys likely exhausted — cycling faster won't help).
    fn rotate_key(&self) {
        if self.keys.is_empty() {
            return;
        }
        let mut last = self.last_rotation.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        *last = std::time::Instant::now();
        drop(last);

        let old = self.key_index.fetch_add(1, Ordering::Relaxed);
        let new_idx = (old + 1) % self.keys.len();
        if self.keys.len() > 1 {
            info!(
                new_index = new_idx,
                total_keys = self.keys.len(),
                "Rotated API key"
            );
        }
    }

    /// Build the JSON body for the Anthropic Messages API.
    fn build_request_body(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<serde_json::Value, Temm1eError> {
        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_tokens.unwrap_or_else(|| {
                let (_, max_output) = temm1e_core::types::model_registry::model_limits(&request.model);
                max_output as u32
            }),
        });

        // P2: emit `system` as an array of text blocks with `cache_control:
        // ephemeral` on the stable base. Volatile tail (if present) goes in a
        // second uncached block. Single block when volatile is absent.
        match (&request.system, &request.system_volatile) {
            (Some(base), Some(vol)) if !vol.is_empty() => {
                body["system"] = serde_json::json!([
                    {"type": "text", "text": base, "cache_control": {"type": "ephemeral"}},
                    {"type": "text", "text": vol},
                ]);
            }
            (Some(base), _) => {
                body["system"] = serde_json::json!([
                    {"type": "text", "text": base, "cache_control": {"type": "ephemeral"}},
                ]);
            }
            (None, Some(vol)) if !vol.is_empty() => {
                // No base but volatile present — emit as single uncached block.
                body["system"] = serde_json::json!([
                    {"type": "text", "text": vol},
                ]);
            }
            _ => {}
        }

        // Preserve system instructions carried in history, rather than filtering
        // them out without forwarding. Only the stable base has a cache breakpoint.
        let mut system = body
            .get("system")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for message in request
            .messages
            .iter()
            .filter(|message| matches!(message.role, Role::System))
        {
            match &message.content {
                MessageContent::Text(text) => {
                    system.push(serde_json::json!({"type":"text","text":text}))
                }
                MessageContent::Parts(parts) => {
                    for part in parts {
                        if let ContentPart::Text { text } = part {
                            system.push(serde_json::json!({"type":"text","text":text}));
                        } else {
                            return Err(Temm1eError::Provider(
                                "System history contains a non-text block".into(),
                            ));
                        }
                    }
                }
            }
        }
        if !system.is_empty() {
            body["system"] = system.into();
        }

        // These catalogued newer models use adaptive thinking by default and
        // reject temperature controls. Existing model defaults are unchanged.
        if matches!(
            request.model.as_str(),
            "claude-fable-5-1" | "claude-opus-5" | "claude-sonnet-5"
        ) {
            body["thinking"] = serde_json::json!({"type":"adaptive"});
        } else if let Some(temp) = request.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if !request.tools.is_empty() {
            let tools: Vec<serde_json::Value> = request
                .tools
                .iter()
                .map(convert_tool_to_anthropic)
                .collect();
            body["tools"] = serde_json::json!(tools);
        }

        let owner = crate::anthropic_native::route(&self.base_url);
        let mut messages = Vec::new();
        for message in request
            .messages
            .iter()
            .filter(|message| !matches!(message.role, Role::System))
        {
            if let MessageContent::Parts(parts) = &message.content {
                let prefix = crate::anthropic_native::context_fingerprint(&body, &messages);
                if let Some(output) =
                    crate::anthropic_native::replay(parts, &owner, &request.model, &prefix)?
                {
                    if !matches!(message.role, Role::Assistant) {
                        return Err(Temm1eError::Provider(
                            "Native Anthropic state belongs only to assistant messages".into(),
                        ));
                    }
                    messages.push(serde_json::json!({"role":"assistant","content":output}));
                    continue;
                }
            }
            messages.push(convert_message_to_anthropic(message)?);
        }
        body["messages"] = messages.into();

        if stream {
            body["stream"] = serde_json::json!(true);
        }

        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// Anthropic API serde types (response)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    id: String,
    content: Vec<serde_json::Value>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
}

impl TryFrom<AnthropicUsage> for Usage {
    type Error = Temm1eError;
    fn try_from(raw: AnthropicUsage) -> Result<Self, Self::Error> {
        Ok(Self {
            totals_reported: Some(true),
            input_tokens: raw
                .input_tokens
                .checked_add(raw.cache_read_input_tokens.unwrap_or(0))
                .and_then(|n| n.checked_add(raw.cache_creation_input_tokens.unwrap_or(0)))
                .ok_or_else(|| Temm1eError::Provider("Anthropic usage total overflow".into()))?,
            output_tokens: raw.output_tokens,
            cache_read_tokens: raw.cache_read_input_tokens,
            cache_write_tokens: raw.cache_creation_input_tokens,
            cost_usd: 0.0,
        })
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn convert_message_to_anthropic(msg: &ChatMessage) -> Result<serde_json::Value, Temm1eError> {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "user", // tool results are sent as user messages in Anthropic API
        Role::System => {
            // System messages are handled separately; skip here.
            return Err(Temm1eError::Provider(
                "System role should not appear in messages list".into(),
            ));
        }
    };

    let content = match &msg.content {
        MessageContent::Text(text) => {
            if matches!(msg.role, Role::Tool) {
                // Shouldn't normally hit here, but handle gracefully
                serde_json::json!(text)
            } else {
                serde_json::json!(text)
            }
        }
        MessageContent::Parts(parts) => {
            let blocks: Vec<serde_json::Value> = parts
                .iter()
                .filter_map(|p| {
                    Some(match p {
                        ContentPart::ProviderState { .. } => return None,
                        ContentPart::Text { text } => serde_json::json!({
                            "type": "text",
                            "text": text,
                        }),
                        ContentPart::ToolUse {
                            id, name, input, ..
                        } => serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        }),
                        ContentPart::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content,
                            "is_error": is_error,
                        }),
                        ContentPart::Image { media_type, data } => serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": data,
                            },
                        }),
                    })
                })
                .collect();
            serde_json::json!(blocks)
        }
    };

    Ok(serde_json::json!({
        "role": role,
        "content": content,
    }))
}

fn convert_tool_to_anthropic(tool: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.parameters,
    })
}

// ---------------------------------------------------------------------------
// Provider trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        let body = self.build_request_body(&request, false)?;

        debug!(provider = "anthropic", model = %request.model, "Sending completion request");

        // Rate-limit retry loop. Non-429 errors return immediately; success
        // returns immediately; 429 backs off and retries up to MAX_RATELIMIT_RETRIES.
        for attempt in 0..=crate::rate_limit::MAX_RATELIMIT_RETRIES {
            let api_key = self.current_key().to_string();
            let response = self
                .client
                .post(format!("{}/v1/messages", self.base_url))
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| Temm1eError::Provider(format!("Anthropic request failed: {e}")))?;

            let status = response.status();

            // 429 handling: wait then retry, unless retries exhausted.
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait = crate::rate_limit::parse_retry_after(&response)
                    .unwrap_or_else(|| crate::rate_limit::default_backoff(attempt));
                let error_body = format!("Anthropic request rejected ({status}); check credentials, quota and model parameters");
                self.rotate_key();
                if attempt == crate::rate_limit::MAX_RATELIMIT_RETRIES
                    || wait > crate::rate_limit::MAX_INLINE_WAIT
                {
                    error!(
                        provider = "anthropic",
                        attempts = attempt + 1,
                        "Rate limit: retries exhausted"
                    );
                    return Err(Temm1eError::RateLimited(format!(
                        "Retry after at least {} seconds; {error_body}",
                        wait.as_secs()
                    )));
                }
                tracing::warn!(
                    provider = "anthropic",
                    attempt = attempt + 1,
                    wait_ms = wait.as_millis() as u64,
                    "Rate limited, backing off before retry"
                );
                tokio::time::sleep(wait).await;
                continue;
            }

            if !status.is_success() {
                let error_body = format!("Anthropic request rejected ({status}); check credentials, quota and model parameters");
                error!(provider = "anthropic", %status, "API error: {}", error_body);
                if status == reqwest::StatusCode::UNAUTHORIZED {
                    self.rotate_key();
                    return Err(Temm1eError::Auth(error_body));
                }
                return Err(Temm1eError::Provider(format!(
                    "Anthropic API error ({status}): {error_body}"
                )));
            }

            let mut bytes = Vec::new();
            let mut chunks = response.bytes_stream();
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk
                    .map_err(|_| Temm1eError::Provider("Anthropic response interrupted".into()))?;
                if bytes.len().saturating_add(chunk.len()) > 32 * 1024 * 1024 {
                    return Err(Temm1eError::Provider(
                        "Anthropic response exceeds 32 MiB".into(),
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            let api_response: AnthropicResponse = serde_json::from_slice(&bytes)
                .map_err(|_| Temm1eError::Provider("Invalid Anthropic response".into()))?;
            let content = crate::anthropic_native::completion(
                api_response.id.clone(),
                api_response.content,
                &crate::anthropic_native::route(&self.base_url),
                &request.model,
                Some(crate::anthropic_native::context_fingerprint(
                    &body,
                    body["messages"].as_array().expect("built message array"),
                )),
            )?;
            if api_response.stop_reason.is_none()
                || (content
                    .iter()
                    .any(|part| matches!(part, ContentPart::ToolUse { .. }))
                    && api_response.stop_reason.as_deref() != Some("tool_use"))
            {
                return Err(Temm1eError::Provider(
                    "Anthropic response did not confirm complete tool generation".into(),
                ));
            }

            return Ok(CompletionResponse {
                id: api_response.id,
                content,
                stop_reason: api_response.stop_reason,
                usage: api_response.usage.try_into()?,
            });
        }

        // Unreachable: loop either returns a value or exhausts and returns
        // RateLimited when attempt == MAX_RATELIMIT_RETRIES.
        unreachable!("rate-limit retry loop must exit via return")
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        let body = self.build_request_body(&request, true)?;

        debug!(provider = "anthropic", model = %request.model, "Sending streaming request");

        // Rate-limit retry at REQUEST-INITIATION ONLY. Once the byte stream
        // begins yielding, a 429 mid-stream cannot be safely retried because
        // SSE parser state (the unfold closure) is not recoverable.
        let response = 'retry: {
            for attempt in 0..=crate::rate_limit::MAX_RATELIMIT_RETRIES {
                let api_key = self.current_key().to_string();
                let response = self
                    .client
                    .post(format!("{}/v1/messages", self.base_url))
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| {
                        Temm1eError::Provider(format!("Anthropic stream request failed: {e}"))
                    })?;

                let status = response.status();
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let wait = crate::rate_limit::parse_retry_after(&response)
                        .unwrap_or_else(|| crate::rate_limit::default_backoff(attempt));
                    let error_body = format!("Anthropic request rejected ({status}); check credentials, quota and model parameters");
                    self.rotate_key();
                    if attempt == crate::rate_limit::MAX_RATELIMIT_RETRIES
                        || wait > crate::rate_limit::MAX_INLINE_WAIT
                    {
                        error!(
                            provider = "anthropic",
                            attempts = attempt + 1,
                            "Rate limit (stream): retries exhausted"
                        );
                        return Err(Temm1eError::RateLimited(format!(
                            "Retry after at least {} seconds; {error_body}",
                            wait.as_secs()
                        )));
                    }
                    tracing::warn!(
                        provider = "anthropic",
                        attempt = attempt + 1,
                        wait_ms = wait.as_millis() as u64,
                        "Rate limited (stream), backing off before retry"
                    );
                    tokio::time::sleep(wait).await;
                    continue;
                }

                if !status.is_success() {
                    let error_body = format!("Anthropic request rejected ({status}); check credentials, quota and model parameters");
                    if status == reqwest::StatusCode::UNAUTHORIZED {
                        self.rotate_key();
                        return Err(Temm1eError::Auth(error_body));
                    }
                    return Err(Temm1eError::Provider(format!(
                        "Anthropic API error ({status}): {error_body}"
                    )));
                }

                break 'retry response;
            }
            unreachable!("rate-limit retry loop must exit via return or break")
        };

        Ok(crate::anthropic_stream::stream(
            response,
            crate::anthropic_native::route(&self.base_url),
            request.model,
            crate::anthropic_native::context_fingerprint(
                &body,
                body["messages"].as_array().expect("built message array"),
            ),
        ))
    }

    async fn complete_with_observer(
        &self,
        request: CompletionRequest,
        observer: temm1e_core::streaming::TextObserver,
    ) -> Result<CompletionResponse, Temm1eError> {
        temm1e_core::streaming::collect_completion(self.stream(request).await?, observer).await
    }

    async fn health_check(&self) -> Result<bool, Temm1eError> {
        let resp = self
            .client
            .head(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", self.current_key())
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(|e| Temm1eError::Provider(format!("Health check failed: {e}")))?;

        // Anthropic may return 405 for HEAD which still means the server is reachable
        Ok(resp.status().is_success() || resp.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED)
    }

    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        Ok(vec![
            "claude-opus-4-6".to_string(),
            "claude-sonnet-4-6".to_string(),
            "claude-haiku-4-5-20251001".to_string(),
            "claude-3-5-sonnet-20241022".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ])
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn fable_replay_keeps_valid_thinking_and_drops_only_context_invalidated_blocks() {
        let provider = AnthropicProvider::new("fixture".into());
        let mut request = CompletionRequest {
            model: "claude-fable-5-1".into(),
            system: Some("stable rules".into()),
            system_volatile: None,
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("read a".into()),
            }],
            tools: vec![],
            max_tokens: Some(4096),
            temperature: Some(0.3),
        };
        let initial = provider.build_request_body(&request, false).unwrap();
        assert!(initial.get("temperature").is_none());
        assert_eq!(initial["thinking"]["type"], "adaptive");
        let native = vec![
            serde_json::json!({"type":"thinking","thinking":"","signature":"opaque"}),
            serde_json::json!({"type":"redacted_thinking","data":"opaque-redacted"}),
            serde_json::json!({"type":"tool_use","id":"t1","name":"read","input":{"path":"a"}}),
        ];
        let parts = crate::anthropic_native::completion(
            "m1".into(),
            native.clone(),
            &crate::anthropic_native::route(&provider.base_url),
            &request.model,
            Some(crate::anthropic_native::context_fingerprint(
                &initial,
                initial["messages"].as_array().unwrap(),
            )),
        )
        .unwrap();
        request.messages.push(ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Parts(parts),
        });
        request.messages.push(ChatMessage {
            role: Role::Tool,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "t1".into(),
                content: "file content".into(),
                is_error: false,
            }]),
        });
        let history_before = serde_json::to_value(&request.messages).unwrap();
        let unchanged = provider.build_request_body(&request, false).unwrap();
        assert_eq!(
            unchanged["messages"][1]["content"],
            serde_json::json!(native)
        );
        request.system_volatile = Some("changed volatile context".into());
        let changed = provider.build_request_body(&request, false).unwrap();
        assert_eq!(
            changed["messages"][1]["content"],
            serde_json::json!([native[2].clone()])
        );
        assert_eq!(
            serde_json::to_value(&request.messages).unwrap(),
            history_before
        );
        request.messages.push(ChatMessage {
            role: Role::System,
            content: MessageContent::Text("retain this constraint".into()),
        });
        let with_system_history = provider.build_request_body(&request, false).unwrap();
        assert!(with_system_history["system"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["text"] == "retain this constraint"));
    }

    #[test]
    fn cache_usage_overflow_is_rejected() {
        let raw: super::AnthropicUsage = serde_json::from_value(serde_json::json!({
            "input_tokens": u32::MAX, "output_tokens": 1,
            "cache_read_input_tokens": 1
        }))
        .unwrap();
        assert!(super::Usage::try_from(raw).is_err());
    }

    #[test]
    fn cache_usage_is_additive_and_missing_is_unknown() {
        let raw: super::AnthropicUsage = serde_json::from_value(serde_json::json!({
            "input_tokens": 10, "output_tokens": 20,
            "cache_read_input_tokens": 100, "cache_creation_input_tokens": 30
        }))
        .unwrap();
        let usage: super::Usage = raw.try_into().unwrap();
        assert_eq!(usage.input_tokens, 140);
        assert_eq!(usage.cache_read_tokens, Some(100));
        assert_eq!(usage.cache_write_tokens, Some(30));
        let raw: super::AnthropicUsage = serde_json::from_value(serde_json::json!({
            "input_tokens": 10, "output_tokens": 20
        }))
        .unwrap();
        let usage: super::Usage = raw.try_into().unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.cache_read_tokens, None);
        assert_eq!(usage.cache_write_tokens, None);
    }
    use super::*;

    #[test]
    fn build_request_body_basic() {
        let provider = AnthropicProvider::new("test-key".to_string());
        let request = CompletionRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Hello".to_string()),
            }],
            tools: Vec::new(),
            max_tokens: Some(1024),
            temperature: Some(0.5),
            system: Some("Be helpful".to_string()),
            system_volatile: None,
        };

        let body = provider.build_request_body(&request, false).unwrap();
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["temperature"], 0.5);
        // P2: system is emitted as an array of blocks with cache_control on base.
        let system = body["system"].as_array().expect("system should be array");
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "Be helpful");
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn system_emits_cache_control_on_base_only() {
        // P2: with volatile tail, base block gets cache_control, volatile does not.
        let provider = AnthropicProvider::new("k".to_string());
        let request = CompletionRequest {
            model: "m".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Hi".to_string()),
            }],
            tools: Vec::new(),
            max_tokens: Some(1024),
            temperature: None,
            system: Some("stable base".to_string()),
            system_volatile: Some("per-turn mutation".to_string()),
        };

        let body = provider.build_request_body(&request, false).unwrap();
        let system = body["system"].as_array().expect("array");
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["text"], "stable base");
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(system[1]["text"], "per-turn mutation");
        assert!(
            system[1].get("cache_control").is_none(),
            "volatile block must NOT carry cache_control"
        );
    }

    #[test]
    fn system_absent_is_no_system_block() {
        let provider = AnthropicProvider::new("k".to_string());
        let request = CompletionRequest {
            model: "m".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Hi".to_string()),
            }],
            tools: Vec::new(),
            max_tokens: Some(1024),
            temperature: None,
            system: None,
            system_volatile: None,
        };

        let body = provider.build_request_body(&request, false).unwrap();
        assert!(body.get("system").is_none());
    }

    #[test]
    fn build_request_body_with_stream() {
        let provider = AnthropicProvider::new("key".to_string());
        let request = CompletionRequest {
            model: "m".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Hi".to_string()),
            }],
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
            system: None,
            system_volatile: None,
        };

        let body = provider.build_request_body(&request, true).unwrap();
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn build_request_body_with_tools() {
        let provider = AnthropicProvider::new("key".to_string());
        let request = CompletionRequest {
            model: "m".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Hi".to_string()),
            }],
            tools: vec![ToolDefinition {
                name: "shell".to_string(),
                description: "Run shell commands".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            max_tokens: None,
            temperature: None,
            system: None,
            system_volatile: None,
        };

        let body = provider.build_request_body(&request, false).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "shell");
        assert_eq!(tools[0]["input_schema"]["type"], "object");
    }

    #[test]
    fn convert_message_filters_system() {
        let msg = ChatMessage {
            role: Role::System,
            content: MessageContent::Text("system prompt".to_string()),
        };
        let result = convert_message_to_anthropic(&msg);
        assert!(result.is_err());
    }

    #[test]
    fn convert_message_user_text() {
        let msg = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Hello".to_string()),
        };
        let json = convert_message_to_anthropic(&msg).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Hello");
    }

    #[test]
    fn convert_message_tool_role_becomes_user() {
        let msg = ChatMessage {
            role: Role::Tool,
            content: MessageContent::Text("tool result".to_string()),
        };
        let json = convert_message_to_anthropic(&msg).unwrap();
        assert_eq!(json["role"], "user");
    }

    #[test]
    fn convert_tool_definition() {
        let tool = ToolDefinition {
            name: "browser".to_string(),
            description: "Browse the web".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        };
        let json = convert_tool_to_anthropic(&tool);
        assert_eq!(json["name"], "browser");
        assert_eq!(json["description"], "Browse the web");
        assert!(json["input_schema"].is_object());
    }

    #[test]
    fn convert_message_with_image() {
        let msg = ChatMessage {
            role: Role::User,
            content: MessageContent::Parts(vec![
                ContentPart::Text {
                    text: "What is in this image?".to_string(),
                },
                ContentPart::Image {
                    media_type: "image/jpeg".to_string(),
                    data: "abc123base64".to_string(),
                },
            ]),
        };
        let json = convert_message_to_anthropic(&msg).unwrap();
        assert_eq!(json["role"], "user");
        let content = json["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["type"], "base64");
        assert_eq!(content[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(content[1]["source"]["data"], "abc123base64");
    }

    #[test]
    fn anthropic_provider_name() {
        let provider = AnthropicProvider::new("key".to_string());
        assert_eq!(provider.name(), "anthropic");
    }

    #[test]
    fn anthropic_with_base_url() {
        let provider = AnthropicProvider::new("key".to_string())
            .with_base_url("https://custom.api.com".to_string());
        assert_eq!(provider.base_url, "https://custom.api.com");
    }

    #[test]
    fn thinking_block_is_not_display_text() {
        let json = r#"{"type": "thinking", "thinking": "some reasoning"}"#;
        let block: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(crate::anthropic_native::normalize(&[block])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn mixed_response_normalizes_visible_text_without_exposing_thinking() {
        let json = r#"{
            "id": "msg_test",
            "content": [
                {"type": "text", "text": "Hello"},
                {"type": "thinking", "thinking": "reasoning about stuff"},
                {"type": "text", "text": "World"}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 20}
        }"#;
        let response: AnthropicResponse = serde_json::from_str(json).unwrap();
        let parts = crate::anthropic_native::normalize(&response.content).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], ContentPart::Text { text } if text == "Hello"));
        assert!(matches!(&parts[1], ContentPart::Text { text } if text == "World"));
    }

    #[test]
    fn standard_anthropic_response_parses_unchanged() {
        // Verify a typical Anthropic response (text + tool_use, no unknown blocks)
        // still deserializes and converts correctly after adding #[serde(other)].
        let json = r#"{
            "id": "msg_abc123",
            "content": [
                {"type": "text", "text": "I'll look that up for you."},
                {"type": "tool_use", "id": "toolu_01", "name": "search", "input": {"query": "rust serde"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 50, "output_tokens": 30}
        }"#;
        let response: AnthropicResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.id, "msg_abc123");
        assert_eq!(response.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(response.usage.input_tokens, 50);
        assert_eq!(response.usage.output_tokens, 30);

        let parts = crate::anthropic_native::normalize(&response.content).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(
            matches!(&parts[0], ContentPart::Text { text } if text == "I'll look that up for you.")
        );
        assert!(
            matches!(&parts[1], ContentPart::ToolUse { id, name, .. } if id == "toolu_01" && name == "search")
        );
    }
}
