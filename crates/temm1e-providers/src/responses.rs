//! Stateless Responses codec shared by API-key and Codex subscription transports.
//! Keep native output items (including encrypted reasoning and phase) intact.
use crate::sse_transport::Protocol;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use temm1e_core::{
    sse::SseEvent,
    types::{error::Temm1eError, message::*},
};

fn error(message: &str) -> Temm1eError {
    Temm1eError::Provider(format!("Responses protocol: {message}"))
}

pub fn build_request(
    request: &CompletionRequest,
    provider: &str,
    codex: bool,
) -> Result<Value, Temm1eError> {
    let mut instructions = request.system_flattened().into_iter().collect::<Vec<_>>();
    let mut input = Vec::new();
    for message in &request.messages {
        if matches!(message.role, Role::System) {
            match &message.content {
                MessageContent::Text(text) => instructions.push(text.clone()),
                MessageContent::Parts(parts) => {
                    for part in parts {
                        if let ContentPart::Text { text } = part {
                            instructions.push(text.clone());
                        }
                    }
                }
            }
            continue;
        }
        if let MessageContent::Parts(parts) = &message.content {
            let native: Vec<_> = parts
                .iter()
                .filter_map(|part| match part {
                    ContentPart::ProviderState {
                        provider: owner,
                        model,
                        output,
                        ..
                    } if owner == provider && model == &request.model => Some(output),
                    _ => None,
                })
                .collect();
            if !native.is_empty() {
                if native.len() != 1 || !matches!(message.role, Role::Assistant) {
                    return Err(error("native state must belong to one assistant response"));
                }
                let original = native[0];
                validate_output(original)?;
                // A pruned/edited tool representation must not resurrect a
                // hidden call from opaque replay state.
                let plain_calls: Vec<_> = parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::ToolUse {
                            id, name, input, ..
                        } => Some((id.clone(), name.clone(), input.clone())),
                        _ => None,
                    })
                    .collect();
                let native_calls = tool_calls(original)?;
                if plain_calls != native_calls {
                    return Err(error("native and normalized tool history disagree"));
                }
                input.extend(original.iter().cloned());
                let visible = parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let original_text = output_text(original)?;
                if visible != original_text && !visible.is_empty() {
                    // Witness/redaction can change delivery text. Preserve the
                    // model's wire state and the actual delivered correction.
                    input.push(json!({"role":"assistant", "phase":"final_answer",
                        "content": format!("Delivered reply after harness processing:\n{visible}")}));
                }
                continue;
            }
        }
        let role = match message.role {
            Role::Assistant => "assistant",
            Role::User => "user",
            Role::Tool => "tool",
            Role::System => unreachable!(),
        };
        match &message.content {
            MessageContent::Text(text) => {
                if role == "tool" {
                    return Err(error("tool result is missing a call ID"));
                }
                input.push(json!({"role":role,"content":text}));
            }
            MessageContent::Parts(parts) => {
                let mut content = Vec::new();
                for part in parts {
                    match part {
                        ContentPart::Text { text } => content.push(json!({"type":if role == "assistant" {"output_text"} else {"input_text"},"text":text})),
                        ContentPart::Image { media_type, data } if role == "user" => content.push(json!({"type":"input_image","image_url":format!("data:{media_type};base64,{data}")})),
                        ContentPart::ToolUse { id, name, input: args, .. } if role == "assistant" => {
                            if id.is_empty() || name.is_empty() || !args.is_object() { return Err(error("invalid historical function call")); }
                            if !content.is_empty() {
                                input.push(json!({"role":role,"content":std::mem::take(&mut content)}));
                            }
                            input.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}));
                        }
                        ContentPart::ToolResult { tool_use_id, content, is_error } if role == "tool" => {
                            if tool_use_id.is_empty() { return Err(error("empty tool result call ID")); }
                            input.push(json!({"type":"function_call_output","call_id":tool_use_id,
                                "output":if *is_error {format!("Tool error: {content}")} else {content.clone()}}));
                        }
                        ContentPart::ProviderState { .. } => {}, // Different route/model: never replay its opaque data.
                        _ => return Err(error("content part is invalid for its message role")),
                    }
                }
                if !content.is_empty() {
                    if role == "tool" {
                        return Err(error("tool text must have a call ID"));
                    }
                    input.push(json!({"role":role,"content":content}));
                }
            }
        }
    }
    let mut pending = HashSet::new();
    let mut seen = HashSet::new();
    for item in &input {
        match item["type"].as_str() {
            Some("function_call") => {
                let id = item["call_id"]
                    .as_str()
                    .ok_or_else(|| error("call ID missing"))?;
                if !seen.insert(id.to_owned()) {
                    return Err(error("duplicate historical call ID"));
                }
                pending.insert(id.to_owned());
            }
            Some("function_call_output") => {
                let id = item["call_id"]
                    .as_str()
                    .ok_or_else(|| error("result ID missing"))?;
                if !pending.remove(id) {
                    return Err(error("unmatched historical tool result"));
                }
            }
            _ => {}
        }
    }
    if !pending.is_empty() {
        return Err(error("historical function call lacks a result"));
    }
    let mut body = json!({"model":request.model,"input":input,"instructions":instructions.join("\n\n"),"stream":true,"store":false,"include":["reasoning.encrypted_content"]});
    if !codex {
        if let Some(max) = request.max_tokens {
            body["max_output_tokens"] = json!(max);
        }
        // This native route currently serves reasoning models whose sampling
        // parameters are unsupported or depend on reasoning configuration.
        // Do not forward the harness's generic temperature default.
    }
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.iter().map(|tool| json!({"type":"function","name":tool.name,"description":tool.description,"parameters":tool.parameters,"strict":false})).collect());
    }
    Ok(body)
}

type NativeCall = (String, String, Value);

fn tool_calls(output: &[Value]) -> Result<Vec<NativeCall>, Temm1eError> {
    let mut calls = Vec::new();
    let mut ids = HashSet::new();
    for item in output.iter().filter(|item| item["type"] == "function_call") {
        let id = item["call_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| error("empty function call ID"))?;
        let name = item["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| error("empty function name"))?;
        let args = item["arguments"]
            .as_str()
            .ok_or_else(|| error("function arguments are not a JSON string"))?;
        if args.len() > 2 * 1024 * 1024 || calls.len() >= 128 || !ids.insert(id) {
            return Err(error("function limits or duplicate ID"));
        }
        let args: Value =
            serde_json::from_str(args).map_err(|_| error("malformed function arguments"))?;
        if !args.is_object() {
            return Err(error("function arguments must be an object"));
        }
        calls.push((id.to_owned(), name.to_owned(), args));
    }
    Ok(calls)
}

fn output_text(output: &[Value]) -> Result<String, Temm1eError> {
    let mut texts = Vec::new();
    for item in output.iter().filter(|item| item["type"] == "message") {
        let content = item["content"]
            .as_array()
            .ok_or_else(|| error("message content is not an array"))?;
        for part in content {
            match part["type"].as_str() {
                Some("output_text") => texts.push(
                    part["text"]
                        .as_str()
                        .ok_or_else(|| error("missing output text"))?
                        .to_owned(),
                ),
                Some("refusal") => texts.push(
                    part["refusal"]
                        .as_str()
                        .ok_or_else(|| error("missing refusal"))?
                        .to_owned(),
                ),
                _ => return Err(error("unsupported message content type")),
            }
        }
    }
    Ok(texts.join(""))
}

fn validate_output(output: &[Value]) -> Result<(), Temm1eError> {
    if output.len() > 1024 {
        return Err(error("too many output items"));
    }
    for item in output {
        match item["type"].as_str() {
            Some("reasoning") => {}
            Some("message") if item["role"] == "assistant" => {}
            Some("function_call") => {}
            _ => return Err(error("unsupported output item; no fabricated completion")),
        }
    }
    tool_calls(output)?;
    output_text(output)?;
    Ok(())
}

pub struct ResponsesProtocol {
    provider: String,
    model: String,
    id: Option<String>,
    sequence: Option<u64>,
    text: String,
    item_ids: HashSet<String>,
    arguments: HashMap<String, String>,
    argument_bytes: usize,
    queue: VecDeque<StreamChunk>,
    completed: bool,
}

impl ResponsesProtocol {
    pub fn new(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            id: None,
            sequence: None,
            text: String::new(),
            item_ids: HashSet::new(),
            arguments: HashMap::new(),
            argument_bytes: 0,
            queue: VecDeque::new(),
            completed: false,
        }
    }
    fn chunk(&self) -> StreamChunk {
        StreamChunk {
            provider_state: None,
            usage: None,
            response_id: self.id.clone(),
            delta: None,
            tool_use: None,
            stop_reason: None,
        }
    }
}

impl Protocol for ResponsesProtocol {
    fn accept(&mut self, event: SseEvent) -> Result<(), Temm1eError> {
        if self.completed {
            return Err(error("event after completion"));
        }
        if event.data == "[DONE]" {
            return Err(error("DONE without confirmed response.completed"));
        }
        let data: Value =
            serde_json::from_str(&event.data).map_err(|_| error("malformed SSE JSON"))?;
        let kind = data["type"]
            .as_str()
            .or_else(|| (!event.event.is_empty()).then_some(event.event.as_str()))
            .ok_or_else(|| error("missing event type"))?;
        if let (Some(wire), Some(json)) = (
            (!event.event.is_empty()).then_some(event.event.as_str()),
            data["type"].as_str(),
        ) {
            if wire != json && wire != "message" {
                return Err(error("SSE event/type mismatch"));
            }
        }
        if data.get("sequence_number").is_some() && data["sequence_number"].as_u64().is_none() {
            return Err(error("invalid sequence number"));
        }
        if let Some(seq) = data["sequence_number"].as_u64() {
            if self.sequence.is_some_and(|previous| seq <= previous) {
                return Err(error("replayed or out-of-order event"));
            }
            self.sequence = Some(seq);
        }
        match kind {
            "response.created" | "response.in_progress" | "response.completed" => {
                let response = &data["response"];
                let id = response["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| error("missing response ID"))?;
                if self.id.as_deref().is_some_and(|previous| previous != id) {
                    return Err(error("response ID changed"));
                }
                self.id = Some(id.into());
                if kind == "response.completed" {
                    if response["status"] != "completed" {
                        return Err(error("terminal response was not completed"));
                    }
                    let output = response["output"]
                        .as_array()
                        .ok_or_else(|| error("missing terminal output"))?;
                    validate_output(output)?;
                    let final_ids: HashSet<_> = output
                        .iter()
                        .filter_map(|item| item["id"].as_str())
                        .collect();
                    if self
                        .item_ids
                        .iter()
                        .any(|id| !final_ids.contains(id.as_str()))
                    {
                        return Err(error("terminal output omitted an observed item"));
                    }
                    for item in output {
                        if let Some(delta) =
                            item["id"].as_str().and_then(|id| self.arguments.get(id))
                        {
                            if item["arguments"].as_str() != Some(delta.as_str()) {
                                return Err(error(
                                    "terminal function arguments disagree with deltas",
                                ));
                            }
                        }
                    }
                    let has_calls = output.iter().any(|item| item["type"] == "function_call");
                    let last_phase = output
                        .iter()
                        .rev()
                        .find(|item| item["type"] == "message")
                        .and_then(|item| item["phase"].as_str());
                    if !has_calls && last_phase == Some("commentary") {
                        return Err(error(
                            "commentary-only response is not a final answer or tool request",
                        ));
                    }
                    let final_text = output_text(output)?;
                    if !final_text.starts_with(&self.text) {
                        return Err(error("terminal text disagrees with streamed text"));
                    }
                    if final_text.len() > self.text.len() {
                        let mut chunk = self.chunk();
                        chunk.delta = Some(final_text[self.text.len()..].into());
                        self.queue.push_back(chunk);
                    }
                    for (id, name, input) in tool_calls(output)? {
                        let mut chunk = self.chunk();
                        chunk.tool_use = Some(ContentPart::ToolUse {
                            id,
                            name,
                            input,
                            thought_signature: None,
                        });
                        self.queue.push_back(chunk);
                    }
                    let mut chunk = self.chunk();
                    chunk.usage = Some(normalize_usage(&response["usage"])?);
                    chunk.stop_reason = Some("end_turn".into());
                    chunk.provider_state = Some(ContentPart::ProviderState {
                        context_fingerprint: None,
                        provider: self.provider.clone(),
                        model: self.model.clone(),
                        response_id: id.into(),
                        output: output.clone(),
                    });
                    self.queue.push_back(chunk);
                    self.completed = true;
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                let delta = data["delta"]
                    .as_str()
                    .ok_or_else(|| error("missing text delta"))?;
                if self.text.len().saturating_add(delta.len()) > 16 * 1024 * 1024 {
                    return Err(error("text exceeds retention limit"));
                }
                self.text.push_str(delta);
                let mut chunk = self.chunk();
                chunk.delta = Some(delta.into());
                self.queue.push_back(chunk);
            }
            "response.output_item.added" => {
                let id = data["item"]["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| error("missing output item ID"))?;
                if self.item_ids.len() >= 1024 || !self.item_ids.insert(id.into()) {
                    return Err(error("duplicate or excessive output items"));
                }
            }
            "response.function_call_arguments.delta" => {
                let id = data["item_id"]
                    .as_str()
                    .ok_or_else(|| error("arguments lack an item ID"))?;
                if !self.item_ids.contains(id) {
                    return Err(error("orphan argument delta"));
                }
                let delta = data["delta"]
                    .as_str()
                    .ok_or_else(|| error("missing arguments delta"))?;
                if self.arguments.len() >= 128 && !self.arguments.contains_key(id) {
                    return Err(error("too many function argument streams"));
                }
                self.argument_bytes = self.argument_bytes.saturating_add(delta.len());
                if self.argument_bytes > 16 * 1024 * 1024 {
                    return Err(error("total function arguments exceed retention limit"));
                }
                let arguments = self.arguments.entry(id.into()).or_default();
                if arguments.len().saturating_add(delta.len()) > 2 * 1024 * 1024 {
                    return Err(error("function arguments exceed limit"));
                }
                arguments.push_str(delta);
            }
            "response.failed" | "response.incomplete" | "error" => {
                return Err(error("provider reported failure or incomplete output"))
            }
            // Preview/done notifications do not authorize dispatch. The terminal
            // output array is the authoritative, validated response snapshot.
            _ => {}
        }
        Ok(())
    }
    fn pop(&mut self) -> Option<StreamChunk> {
        self.queue.pop_front()
    }
    fn done(&self) -> bool {
        self.completed
    }
    fn finish(&self) -> Result<(), Temm1eError> {
        if self.completed {
            Ok(())
        } else {
            Err(error("EOF before response.completed; no tool dispatch"))
        }
    }
}

fn normalize_usage(raw: &Value) -> Result<Usage, Temm1eError> {
    let number = |key: &str| -> Result<Option<u32>, Temm1eError> {
        match raw.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .map(Some)
                .ok_or_else(|| error("invalid usage count")),
        }
    };
    let input = number("input_tokens")?;
    let output = number("output_tokens")?;
    let cached = match raw.pointer("/input_tokens_details/cached_tokens") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| error("invalid cache count"))?,
        ),
    };
    if cached.zip(input).is_some_and(|(read, total)| read > total) {
        return Err(error("cache count exceeds total input"));
    }
    Ok(Usage {
        totals_reported: Some(input.is_some() && output.is_some()),
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or(0),
        cache_read_tokens: cached,
        cache_write_tokens: None,
        cost_usd: 0.0,
    })
}

pub fn stream(
    response: reqwest::Response,
    provider: &str,
    model: &str,
) -> futures::stream::BoxStream<'static, Result<StreamChunk, Temm1eError>> {
    crate::sse_transport::stream(response, ResponsesProtocol::new(provider, model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use temm1e_core::{sse::SseDecoder, streaming::collect_completion};

    fn native_output() -> Vec<Value> {
        vec![
            json!({"id":"rs_1","type":"reasoning","encrypted_content":"opaque-ciphertext","summary":[]}),
            json!({"id":"msg_1","type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"café 🐈","annotations":[]}]}),
            json!({"id":"fc_1","type":"function_call","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"café.txt\"}"}),
        ]
    }
    fn terminal(output: Vec<Value>) -> Value {
        json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","output":output,
            "usage":{"input_tokens":1200,"output_tokens":80,"input_tokens_details":{"cached_tokens":1000}}}})
    }
    fn event(data: Value) -> SseEvent {
        SseEvent {
            event: data["type"].as_str().unwrap().into(),
            data: data.to_string(),
            id: String::new(),
        }
    }
    fn request(messages: Vec<ChatMessage>) -> CompletionRequest {
        CompletionRequest {
            model: "gpt-6-astra".into(),
            messages,
            tools: vec![],
            max_tokens: Some(2048),
            temperature: Some(0.7),
            system: Some("stable".into()),
            system_volatile: Some("active".into()),
        }
    }

    #[tokio::test]
    async fn split_utf8_stream_preserves_usage_and_native_reasoning_without_displaying_it() {
        let events = [
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp_1"}}),
            json!({"type":"response.output_text.delta","sequence_number":1,"delta":"café 🐈"}),
            json!({"type":"response.output_item.added","sequence_number":2,"item":{"id":"fc_1","type":"function_call"}}),
            json!({"type":"response.function_call_arguments.delta","sequence_number":3,"item_id":"fc_1","delta":"{\"path\":\"café.txt\"}"}),
            terminal(native_output()),
        ];
        let wire = events
            .iter()
            .map(|value| {
                format!(
                    "event: {}\r\ndata: {}\r\n\r\n",
                    value["type"].as_str().unwrap(),
                    value
                )
            })
            .collect::<String>();
        let mut sse = SseDecoder::new(2 * 1024 * 1024);
        let mut protocol = ResponsesProtocol::new("openai", "gpt-6-astra");
        for byte in wire.bytes() {
            if let Some(event) = sse.push(byte).unwrap() {
                protocol.accept(event).unwrap();
            }
        }
        sse.finish().unwrap();
        protocol.finish().unwrap();
        let chunks: Vec<_> = std::iter::from_fn(|| protocol.pop()).map(Ok).collect();
        let visible = Arc::new(std::sync::Mutex::new(String::new()));
        let capture = visible.clone();
        let response = collect_completion(
            Box::pin(futures::stream::iter(chunks)),
            Arc::new(move |delta| capture.lock().unwrap().push_str(delta)),
        )
        .await
        .unwrap();
        assert_eq!(*visible.lock().unwrap(), "café 🐈");
        assert_eq!(response.id, "resp_1");
        assert_eq!(response.usage.input_tokens, 1200);
        assert_eq!(response.usage.output_tokens, 80);
        assert_eq!(response.usage.cache_read_tokens, Some(1000));
        assert!(
            matches!(&response.content[2], ContentPart::ProviderState { output, .. } if *output == native_output())
        );
        // Native state survives durable JSON and replays once with its tool result.
        let serialized = serde_json::to_string(&response.content).unwrap();
        let history = vec![
            ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Parts(serde_json::from_str(&serialized).unwrap()),
            },
            ChatMessage {
                role: Role::Tool,
                content: MessageContent::Parts(vec![ContentPart::ToolResult {
                    tool_use_id: "call_1".into(),
                    content: "contents".into(),
                    is_error: false,
                }]),
            },
        ];
        let body = build_request(&request(history.clone()), "openai", false).unwrap();
        assert_eq!(body["input"].as_array().unwrap().len(), 4);
        assert_eq!(&body["input"].as_array().unwrap()[..3], native_output());
        assert_eq!(body["input"][3]["type"], "function_call_output");
        assert_eq!(body["max_output_tokens"], 2048);
        assert!(body.get("temperature").is_none());
        let other = build_request(&request(history), "openai-codex", true).unwrap();
        assert!(!other.to_string().contains("opaque-ciphertext"));
        assert!(other.get("max_output_tokens").is_none());
    }

    #[test]
    fn incomplete_duplicate_and_malformed_tools_never_become_dispatchable() {
        let mut decoder = ResponsesProtocol::new("openai", "gpt-6-astra");
        decoder
            .accept(event(
                json!({"type":"response.output_item.added","item":{"id":"fc_1"}}),
            ))
            .unwrap();
        decoder.accept(event(json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{"}))).unwrap();
        assert!(decoder.pop().is_none());
        assert!(decoder.finish().is_err());
        assert!(decoder.accept(event(terminal(native_output()))).is_err());
        assert!(decoder.pop().is_none());
        let mut commentary = ResponsesProtocol::new("openai", "gpt-6-astra");
        assert!(commentary
            .accept(event(terminal(native_output()[..2].to_vec())))
            .is_err());
        let mut broken = native_output();
        broken[2]["arguments"] = json!("{");
        let mut decoder = ResponsesProtocol::new("openai", "gpt-6-astra");
        assert!(decoder.accept(event(terminal(broken))).is_err());
        assert!(decoder.pop().is_none());
        let mut duplicate = native_output();
        duplicate.push(duplicate[2].clone());
        assert!(validate_output(&duplicate).is_err());
        let mut decoder = ResponsesProtocol::new("openai", "gpt-6-astra");
        assert!(decoder
            .accept(event(json!({"type":"response.incomplete"})))
            .is_err());
        assert!(normalize_usage(
            &json!({"input_tokens":1,"output_tokens":1,"input_tokens_details":{"cached_tokens":2}})
        )
        .is_err());
        assert_eq!(
            normalize_usage(&Value::Null).unwrap().totals_reported,
            Some(false)
        );
    }

    #[test]
    fn tool_schema_is_not_rewritten_and_all_system_segments_survive() {
        let mut request = request(vec![ChatMessage {
            role: Role::System,
            content: MessageContent::Text("second system".into()),
        }]);
        let schema = json!({"type":"object","properties":{"path":{"type":"string"},"limit":{"type":"integer"}},"required":["path"]});
        request.tools.push(ToolDefinition {
            name: "read_file".into(),
            description: "Read".into(),
            parameters: schema.clone(),
        });
        let body = build_request(&request, "openai-codex", true).unwrap();
        assert_eq!(body["instructions"], "stable\n\nactive\n\nsecond system");
        assert_eq!(body["tools"][0]["parameters"], schema);
        assert_eq!(body["tools"][0]["strict"], false);
        assert_eq!(body["store"], false);
    }

    #[test]
    fn pruned_tool_history_does_not_resurrect_native_calls() {
        let native = ContentPart::ProviderState {
            context_fingerprint: None,
            provider: "openai".into(),
            model: "gpt-6-astra".into(),
            response_id: "resp_1".into(),
            output: native_output(),
        };
        let messages = vec![ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![native]),
        }];
        assert!(build_request(&request(messages), "openai", false)
            .unwrap_err()
            .to_string()
            .contains("disagree"));
    }
}
