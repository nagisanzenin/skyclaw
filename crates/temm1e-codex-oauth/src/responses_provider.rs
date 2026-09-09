//! CodexResponsesProvider — Provider trait implementation using the OpenAI Responses API.
//!
//! Uses the shared stateless Responses codec and bounded stream collector.
//! The wire format differs from Chat Completions:
//! - `input` + `instructions` instead of `messages`
//! - `output` items instead of `choices[0].message`
//! - Different tool call schema (function_call / function_call_output)
//! - Different streaming event types

use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::Arc;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::message::*;
use temm1e_core::Provider;

use crate::token_store::TokenStore;

/// Provider that uses OpenAI Responses API with OAuth tokens.
pub struct CodexResponsesProvider {
    token_store: Arc<TokenStore>,
    #[allow(dead_code)]
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl CodexResponsesProvider {
    /// Create a new Codex Responses API provider.
    pub fn new(model: String, token_store: Arc<TokenStore>) -> Self {
        Self {
            token_store,
            model,
            base_url: "https://chatgpt.com/backend-api/codex".to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
        }
    }

    fn build_request_body(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<serde_json::Value, Temm1eError> {
        let mut body = temm1e_providers::responses::build_request(request, "openai-codex", true)?;
        body["stream"] = serde_json::Value::Bool(stream);
        Ok(body)
    }
}

#[async_trait]
impl Provider for CodexResponsesProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        temm1e_core::streaming::collect_completion(self.stream(request).await?, Arc::new(|_| {}))
            .await
    }

    async fn complete_with_observer(
        &self,
        request: CompletionRequest,
        observer: temm1e_core::streaming::TextObserver,
    ) -> Result<CompletionResponse, Temm1eError> {
        temm1e_core::streaming::collect_completion(self.stream(request).await?, observer).await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        let token = self.token_store.get_access_token().await?;
        let body = self.build_request_body(&request, true)?;

        tracing::debug!(model = %request.model, "Codex Responses API request (streaming)");

        let resp = self
            .client
            .post(format!("{}/responses", self.base_url))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                Temm1eError::Provider(format!("Responses API request failed: {}", e.without_url()))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry = temm1e_providers::rate_limit::parse_retry_after(&resp)
                    .map(|wait| format!(" Retry after at least {} seconds.", wait.as_secs()))
                    .unwrap_or_default();
                return Err(Temm1eError::RateLimited(format!(
                    "Codex subscription rate or plan limit reached.{retry}"
                )));
            }
            // Error bodies can echo prompts or credentials. Expose only status.
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(Temm1eError::Auth(format!(
                    "Codex OAuth rejected ({status}); reconnect the subscription account"
                )));
            }
            return Err(Temm1eError::Provider(format!(
                "Codex Responses request failed ({status})"
            )));
        }

        Ok(temm1e_providers::responses::stream(
            resp,
            "openai-codex",
            &request.model,
        ))
    }

    async fn health_check(&self) -> Result<bool, Temm1eError> {
        // Try to get a fresh token — if this works, the OAuth connection is healthy
        match self.token_store.get_access_token().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        // Return the known Codex-compatible models
        Ok(vec![
            "gpt-5.4".to_string(),
            "gpt-5.3-codex".to_string(),
            "gpt-5.3-codex-spark".to_string(),
            "gpt-5.2-codex".to_string(),
            "gpt-5-codex".to_string(),
            "gpt-5-codex-mini".to_string(),
            "gpt-5-mini".to_string(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_sse_data() {
        let data = r#"{"delta":"Hello"}"#;
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(parsed["delta"], "Hello");
    }

    #[test]
    fn parse_tool_call_sse_data() {
        let data = r#"{"call_id":"call_123","name":"shell","delta":"{\"command\": \"ls\"}"}"#;
        let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(parsed["call_id"], "call_123");
        assert_eq!(parsed["name"], "shell");
    }

    #[test]
    fn build_request_extracts_system() {
        let store = Arc::new(TokenStore::new(crate::token_store::CodexOAuthTokens {
            access_token: "test".into(),
            refresh_token: "test".into(),
            expires_at: u64::MAX,
            email: "test@test.com".into(),
            account_id: "org-test".into(),
        }));
        let provider = CodexResponsesProvider::new("gpt-5.3-codex".into(), store);

        let request = CompletionRequest {
            model: "gpt-5.3-codex".into(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Hello".into()),
            }],
            tools: vec![],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            system: Some("You are a helpful assistant.".into()),
            system_volatile: None,
        };

        let body = provider.build_request_body(&request, false).unwrap();
        assert_eq!(body["instructions"], "You are a helpful assistant.");
        let mut volatile_request = request.clone();
        volatile_request.system_volatile = Some("Active goal: preserve this constraint.".into());
        let volatile_body = provider
            .build_request_body(&volatile_request, false)
            .unwrap();
        assert_eq!(
            volatile_body["instructions"],
            "You are a helpful assistant.\n\nActive goal: preserve this constraint."
        );
        volatile_request.system = None;
        let volatile_only = provider
            .build_request_body(&volatile_request, false)
            .unwrap();
        assert_eq!(
            volatile_only["instructions"],
            "Active goal: preserve this constraint."
        );
        assert_eq!(body["model"], "gpt-5.3-codex");
        assert!(body.get("max_output_tokens").is_none());
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn build_request_converts_tools() {
        let store = Arc::new(TokenStore::new(crate::token_store::CodexOAuthTokens {
            access_token: "test".into(),
            refresh_token: "test".into(),
            expires_at: u64::MAX,
            email: "test@test.com".into(),
            account_id: "org-test".into(),
        }));
        let provider = CodexResponsesProvider::new("gpt-5.3-codex".into(), store);

        let request = CompletionRequest {
            model: "gpt-5.3-codex".into(),
            messages: vec![],
            tools: vec![ToolDefinition {
                name: "shell".into(),
                description: "Run a shell command".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"}
                    }
                }),
            }],
            max_tokens: None,
            temperature: None,
            system: None,
            system_volatile: None,
        };

        let body = provider.build_request_body(&request, false).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "shell");
        assert_eq!(tools[0]["strict"], false);
        assert_eq!(tools[0]["parameters"], request.tools[0].parameters);
    }
}
