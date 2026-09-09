//! Bounded generateContent stream. Tools/native state commit only after EOF and
//! a valid terminal candidate; text remains provisional until then.
use crate::{
    gemini_native::{self as native, error},
    sse_transport::Protocol,
};
use serde_json::{json, Value};
use std::collections::VecDeque;
use temm1e_core::{
    sse::SseEvent,
    types::{error::Temm1eError, message::*},
};

pub(crate) struct Decoder {
    owner: String,
    model: String,
    id: String,
    provider_id: Option<String>,
    raw: Vec<Value>,
    bytes: usize,
    stop: Option<String>,
    usage: Value,
    queue: VecDeque<StreamChunk>,
    done: bool,
}
impl Decoder {
    pub(crate) fn new(owner: String, model: String) -> Self {
        Self {
            owner,
            model,
            id: uuid::Uuid::new_v4().to_string(),
            provider_id: None,
            raw: Vec::new(),
            bytes: 0,
            stop: None,
            usage: json!({}),
            queue: VecDeque::new(),
            done: false,
        }
    }
    fn chunk(&self) -> StreamChunk {
        StreamChunk {
            provider_state: None,
            usage: None,
            response_id: self.provider_id.clone(),
            delta: None,
            tool_use: None,
            stop_reason: None,
        }
    }
    pub(crate) fn accept_value(&mut self, value: Value) -> Result<(), Temm1eError> {
        if self.done {
            return Err(error("data after completion"));
        }
        if !value.is_object() || value.get("error").is_some() {
            return Err(error("invalid or failed generation"));
        }
        if value["promptFeedback"].get("blockReason").is_some() {
            return Err(error("prompt blocked by provider"));
        }
        if let Some(id) = value.get("responseId") {
            let id = id
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 1024)
                .ok_or_else(|| error("invalid response identity"))?;
            if self.provider_id.as_deref().is_some_and(|old| old != id) {
                return Err(error("response identity changed"));
            }
            self.provider_id = Some(id.into());
            self.id = id.into();
        }
        if let Some(usage) = value.get("usageMetadata") {
            let usage = usage
                .as_object()
                .ok_or_else(|| error("invalid usage metadata"))?;
            for key in [
                "promptTokenCount",
                "candidatesTokenCount",
                "thoughtsTokenCount",
                "totalTokenCount",
                "cachedContentTokenCount",
            ] {
                if let Some(v) = usage.get(key) {
                    let n = v
                        .as_u64()
                        .filter(|n| *n <= u32::MAX as u64)
                        .ok_or_else(|| error("invalid usage counter"))?;
                    if self.usage[key].as_u64().is_some_and(|old| n < old) {
                        return Err(error("cumulative usage decreased"));
                    }
                    self.usage[key] = v.clone();
                }
            }
        }
        if let Some(candidates) = value.get("candidates") {
            let candidates = candidates
                .as_array()
                .ok_or_else(|| error("invalid candidates"))?;
            if candidates.len() > 1 {
                return Err(error("multiple candidates are unsupported"));
            }
            if let Some(candidate) = candidates.first() {
                if candidate
                    .get("index")
                    .is_some_and(|v| v.as_u64() != Some(0))
                {
                    return Err(error("invalid candidate index"));
                }
                if let Some(content) = candidate.get("content") {
                    if self.stop.is_some() {
                        return Err(error("content after terminal candidate"));
                    }
                    if content.get("role").is_some_and(|r| r != "model") {
                        return Err(error("invalid output role"));
                    }
                    let parts = content["parts"]
                        .as_array()
                        .ok_or_else(|| error("invalid output parts"))?;
                    for part in parts {
                        self.bytes = self.bytes.saturating_add(
                            serde_json::to_vec(part)
                                .map_err(|_| error("invalid output"))?
                                .len(),
                        );
                        if self.bytes > 16 * 1024 * 1024 || self.raw.len() >= 4096 {
                            return Err(error("native response exceeds limit"));
                        }
                        // Validate before display; thought summaries are never final text.
                        let visible = native::normalize(std::slice::from_ref(part), &self.id)?;
                        for item in visible {
                            if let ContentPart::Text { text } = item {
                                let mut chunk = self.chunk();
                                chunk.delta = Some(text);
                                self.queue.push_back(chunk);
                            }
                        }
                        self.raw.push(part.clone());
                    }
                }
                if let Some(stop) = candidate.get("finishReason") {
                    let stop = stop
                        .as_str()
                        .ok_or_else(|| error("invalid finish reason"))?;
                    if self.stop.is_some() {
                        return Err(error("duplicate terminal candidate"));
                    }
                    if !matches!(stop, "STOP" | "MAX_TOKENS") {
                        return Err(error("generation blocked or unsuccessful"));
                    }
                    self.stop = Some(stop.into());
                }
            }
        }
        Ok(())
    }
    pub(crate) fn complete(&self) -> Result<CompletionResponse, Temm1eError> {
        let stop = self
            .stop
            .clone()
            .ok_or_else(|| error("truncated response without terminal candidate"))?;
        if stop != "STOP" && self.raw.iter().any(|p| p.get("functionCall").is_some()) {
            return Err(error("truncated function call is not executable"));
        }
        if self.raw.is_empty() {
            return Err(error("generation returned no supported content"));
        }
        native::completion(
            self.raw.clone(),
            self.id.clone(),
            &self.owner,
            &self.model,
            stop,
            native::usage(&self.usage)?,
        )
    }
}
impl Protocol for Decoder {
    fn accept(&mut self, event: SseEvent) -> Result<(), Temm1eError> {
        let value = serde_json::from_str(&event.data).map_err(|_| error("invalid stream JSON"))?;
        self.accept_value(value)
    }
    fn pop(&mut self) -> Option<StreamChunk> {
        self.queue.pop_front()
    }
    fn done(&self) -> bool {
        self.done
    }
    fn finish(&self) -> Result<(), Temm1eError> {
        self.complete().map(|_| ())
    }
    fn end(&mut self) -> Result<(), Temm1eError> {
        let response = self.complete()?;
        let mut final_state = None;
        for part in response.content {
            let mut chunk = self.chunk();
            chunk.response_id = Some(self.id.clone());
            match part {
                ContentPart::ToolUse { .. } => chunk.tool_use = Some(part),
                ContentPart::ProviderState { .. } => {
                    final_state = Some(part);
                    continue;
                }
                _ => continue,
            }
            self.queue.push_back(chunk);
        }
        let mut chunk = self.chunk();
        chunk.response_id = Some(self.id.clone());
        chunk.provider_state = final_state;
        chunk.stop_reason = response.stop_reason;
        chunk.usage = Some(response.usage);
        self.queue.push_back(chunk);
        self.done = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_and_usage_validation_precede_tool_commit() {
        for response in [
            json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"run","args":{}}}]},"finishReason":"MAX_TOKENS"}]}),
            json!({"candidates":[{"content":{"parts":[{"text":"blocked"}]},"finishReason":"SAFETY"}]}),
            json!({"candidates":[{"content":{"parts":[{"functionCall":{"id":"same","name":"run","args":{}}},{"functionCall":{"id":"same","name":"run","args":{}}}]},"finishReason":"STOP"}]}),
            json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"run","args":{}}}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":20,"totalTokenCount":1}}),
        ] {
            let mut decoder = Decoder::new("route".into(), "model".into());
            assert!(decoder
                .accept_value(response)
                .and_then(|_| decoder.end())
                .is_err());
            while let Some(chunk) = decoder.pop() {
                assert!(chunk.tool_use.is_none());
                assert!(chunk.provider_state.is_none());
            }
        }
    }
}
