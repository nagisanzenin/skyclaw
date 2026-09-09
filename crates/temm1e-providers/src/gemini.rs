//! Native generateContent JSON/SSE transport and explicit function continuation.
use crate::gemini_native::{self as native, error};
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use temm1e_core::{
    types::{error::Temm1eError, message::*},
    Provider,
};

pub struct GeminiProvider {
    api_key: String,
    client: Client,
    base_url: String,
}
impl GeminiProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .connect_timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
        }
    }
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url.trim_end_matches('/').into();
        self
    }
    fn endpoint(&self, model: &str, stream: bool) -> Result<String, Temm1eError> {
        // Model IDs are one resource component, never a URL/path/query escape.
        let model = model.strip_prefix("models/").unwrap_or(model);
        if model.is_empty()
            || model.len() > 256
            || !model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(error("invalid model ID"));
        }
        Ok(format!(
            "{}/models/{}:{}",
            self.base_url,
            model,
            if stream {
                "streamGenerateContent?alt=sse"
            } else {
                "generateContent"
            }
        ))
    }
    fn convert_request(&self, request: &CompletionRequest) -> Result<Value, Temm1eError> {
        let mut system = request.system_flattened().unwrap_or_default();
        let mut contents = Vec::new();
        // Internal IDs map to function name and optional provider-issued ID.
        let mut calls: HashMap<String, (String, Option<String>)> = HashMap::new();
        for message in &request.messages {
            let parts = match &message.content {
                MessageContent::Text(text) => vec![ContentPart::Text { text: text.clone() }],
                MessageContent::Parts(parts) => parts.clone(),
            };
            if matches!(message.role, Role::System) {
                for part in parts {
                    if let ContentPart::Text { text } = part {
                        if !system.is_empty() {
                            system.push('\n');
                        }
                        system.push_str(&text);
                    }
                }
                continue;
            }
            let replay = if matches!(message.role, Role::Assistant) {
                native::replay(&parts, &native::route(&self.base_url), &request.model)?
            } else {
                None
            };
            let mut wire = Vec::new();
            let native_calls: Vec<&Value> = replay
                .as_ref()
                .map(|r| r.iter().filter_map(|p| p.get("functionCall")).collect())
                .unwrap_or_default();
            let mut call_index = 0;
            for part in &parts {
                match part {
                    ContentPart::Text { text } => wire.push(json!({"text":text})),
                    ContentPart::Image { media_type, data } => {
                        wire.push(json!({"inlineData":{"mimeType":media_type,"data":data}}))
                    }
                    ContentPart::ToolUse {
                        id,
                        name,
                        input,
                        thought_signature,
                    } => {
                        if !matches!(message.role, Role::Assistant) {
                            return Err(error("function call outside assistant history"));
                        }
                        let original = native_calls.get(call_index);
                        let native_id = original.and_then(|c| c["id"].as_str()).map(str::to_owned);
                        let wire_name = original
                            .and_then(|c| c["name"].as_str())
                            .unwrap_or(name)
                            .to_owned();
                        if calls.insert(id.clone(), (wire_name, native_id)).is_some() {
                            return Err(error("duplicate function history ID"));
                        }
                        call_index += 1;
                        let mut p = json!({"functionCall":{"name":name,"args":input}});
                        if replay.is_none()
                            && !parts
                                .iter()
                                .any(|p| matches!(p, ContentPart::ProviderState { .. }))
                        {
                            if let Some(sig) = thought_signature {
                                p["thoughtSignature"] = json!(sig);
                            }
                        }
                        wire.push(p);
                    }
                    ContentPart::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        if matches!(message.role, Role::Assistant) {
                            return Err(error("function result in assistant history"));
                        }
                        let (name, native_id) = calls
                            .remove(tool_use_id)
                            .ok_or_else(|| error("orphan or duplicate function result"))?;
                        let mut result = json!({"name":name,"response": if *is_error {json!({"error":content})} else {json!({"result":content})}});
                        if let Some(id) = native_id {
                            result["id"] = json!(id);
                        }
                        wire.push(json!({"functionResponse":result}));
                    }
                    ContentPart::ProviderState { .. } => {}
                }
            }
            if let Some(replay) = replay {
                wire = replay;
            }
            if !wire.is_empty() {
                contents.push(json!({"role":if matches!(message.role, Role::Assistant) {"model"} else {"user"},"parts":wire}));
            }
        }
        if !calls.is_empty() {
            return Err(error("unresolved function calls in request history"));
        }
        if contents.first().is_some_and(|c| c["role"] == "model") {
            contents.insert(0, json!({"role":"user","parts":[{"text":"."}]}));
        }
        let mut body = json!({"contents":contents,"generationConfig":{"candidateCount":1}});
        if !system.is_empty() {
            body["systemInstruction"] = json!({"parts":[{"text":system}]});
        }
        if let Some(t) = request.temperature {
            body["generationConfig"]["temperature"] = json!(t);
        }
        if let Some(n) = request.max_tokens {
            body["generationConfig"]["maxOutputTokens"] = json!(n);
        }
        if !request.tools.is_empty() {
            body["tools"] = json!([{"functionDeclarations":request.tools.iter().map(|t| json!({"name":t.name,"description":t.description,"parameters":strip_unsupported_schema_fields(&t.parameters)})).collect::<Vec<_>>()}]);
        }
        Ok(body)
    }
    async fn send(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> Result<reqwest::Response, Temm1eError> {
        let response = self
            .client
            .post(self.endpoint(&request.model, stream)?)
            .header("x-goog-api-key", &self.api_key)
            .json(&self.convert_request(request)?)
            .send()
            .await
            .map_err(|e| error(&format!("HTTP request failed: {}", e.without_url())))?;
        if !response.status().is_success() {
            return Err(error(&format!("API HTTP status {}", response.status())));
        }
        Ok(response)
    }
}
async fn read_json(response: reqwest::Response) -> Result<Value, Temm1eError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| error(&format!("response read failed: {}", e.without_url())))?;
        if bytes.len().saturating_add(chunk.len()) > 32 * 1024 * 1024 {
            return Err(error("JSON response exceeds 32 MiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| error("invalid JSON response"))
}
#[async_trait]
impl Provider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        let body = read_json(self.send(&request, false).await?).await?;
        let mut decoder =
            crate::gemini_stream::Decoder::new(native::route(&self.base_url), request.model);
        decoder.accept_value(body)?;
        decoder.complete()
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
        let response = self.send(&request, true).await?;
        Ok(crate::sse_transport::stream(
            response,
            crate::gemini_stream::Decoder::new(native::route(&self.base_url), request.model),
        ))
    }
    async fn health_check(&self) -> Result<bool, Temm1eError> {
        let response = self
            .client
            .get(format!("{}/models?pageSize=1", self.base_url))
            .header("x-goog-api-key", &self.api_key)
            .send()
            .await
            .map_err(|e| error(&format!("health request failed: {}", e.without_url())))?;
        Ok(response.status().is_success())
    }
    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        let mut names = Vec::new();
        let mut page = None::<String>;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..32 {
            let mut request = self
                .client
                .get(format!("{}/models", self.base_url))
                .header("x-goog-api-key", &self.api_key)
                .query(&[("pageSize", "100")]);
            if let Some(token) = &page {
                request = request.query(&[("pageToken", token)]);
            }
            let response = request
                .send()
                .await
                .map_err(|e| error(&format!("model list failed: {}", e.without_url())))?;
            if !response.status().is_success() {
                return Err(error(&format!(
                    "model list HTTP status {}",
                    response.status()
                )));
            }
            let body = read_json(response).await?;
            for model in body["models"]
                .as_array()
                .ok_or_else(|| error("invalid model list"))?
            {
                if let Some(name) = model["name"].as_str() {
                    names.push(name.strip_prefix("models/").unwrap_or(name).to_owned());
                }
            }
            page = body["nextPageToken"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let Some(token) = &page else { return Ok(names) };
            if token.len() > 8192 || !seen.insert(token.clone()) {
                return Err(error("invalid model pagination"));
            }
        }
        Err(error("model pagination exceeds 32 pages"))
    }
}
fn strip_unsupported_schema_fields(schema: &serde_json::Value) -> serde_json::Value {
    match schema {
        serde_json::Value::Object(map) => {
            let mut cleaned = serde_json::Map::new();
            for (key, value) in map {
                // Gemini doesn't support these JSON Schema fields
                if key == "additionalProperties" || key == "$schema" {
                    continue;
                }
                cleaned.insert(key.clone(), strip_unsupported_schema_fields(value));
            }
            serde_json::Value::Object(cleaned)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(strip_unsupported_schema_fields).collect())
        }
        other => other.clone(),
    }
}
