//! Incremental Chat Completions decoder. Tool arguments are committed only
//! after the entire call validates; malformed or truncated calls never become {}.
use futures::{stream::BoxStream, StreamExt};
use std::collections::{BTreeMap, VecDeque};
use temm1e_core::{
    sse::SseDecoder,
    types::{error::Temm1eError, message::*},
};

fn error(message: &str) -> Temm1eError {
    Temm1eError::Provider(format!("Chat stream: {message}"))
}
fn count(value: &serde_json::Value, key: &str) -> Result<u32, Temm1eError> {
    value[key]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| error("invalid usage count"))
}
fn optional_count(value: &serde_json::Value, key: &str) -> Result<Option<u32>, Temm1eError> {
    if value.get(key).is_none_or(serde_json::Value::is_null) {
        Ok(None)
    } else {
        count(value, key).map(Some)
    }
}
pub(crate) fn usage(value: &serde_json::Value) -> Result<Usage, Temm1eError> {
    let details = &value["prompt_tokens_details"];
    Ok(Usage {
        totals_reported: Some(true),
        input_tokens: count(value, "prompt_tokens")?,
        output_tokens: count(value, "completion_tokens")?,
        cache_read_tokens: optional_count(details, "cached_tokens")?,
        cache_write_tokens: optional_count(details, "cache_write_tokens")?,
        cost_usd: 0.0,
    })
}
#[derive(Default)]
struct ToolBuffer {
    id: String,
    name: String,
    arguments: String,
}
#[derive(Default)]
struct Decoder {
    id: Option<String>,
    tools: BTreeMap<usize, ToolBuffer>,
    queue: VecDeque<StreamChunk>,
    finished: bool,
    done: bool,
    tool_bytes: usize,
}
impl Decoder {
    fn chunk(&self) -> StreamChunk {
        StreamChunk {
            usage: None,
            response_id: self.id.clone(),
            delta: None,
            tool_use: None,
            stop_reason: None,
        }
    }
    fn accept(&mut self, data: &str) -> Result<(), Temm1eError> {
        if data == "[DONE]" {
            if !self.finished {
                return Err(error("DONE arrived before a completion marker"));
            }
            self.done = true;
            return Ok(());
        }
        let value: serde_json::Value =
            serde_json::from_str(data).map_err(|_| error("invalid JSON event"))?;
        if value.get("error").is_some() {
            return Err(error("provider returned an error event"));
        }
        if let Some(id) = value
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty())
        {
            if self.id.as_deref().is_some_and(|old| old != id) {
                return Err(error("response ID changed"));
            }
            self.id = Some(id.into());
        }
        if let Some(raw) = value.get("usage").filter(|v| !v.is_null()) {
            let mut chunk = self.chunk();
            chunk.usage = Some(usage(raw)?);
            self.queue.push_back(chunk);
        }
        let choices = value["choices"]
            .as_array()
            .ok_or_else(|| error("missing choices array"))?;
        if choices.len() > 1 {
            return Err(error(
                "multiple choices are unsupported for a single agent response",
            ));
        }
        for choice in choices {
            if choice
                .get("index")
                .and_then(|v| v.as_u64())
                .is_some_and(|n| n != 0)
            {
                return Err(error("unexpected choice index"));
            }
            let delta = &choice["delta"];
            if self.finished
                && (delta.get("tool_calls").is_some()
                    || delta["content"].as_str().is_some_and(|s| !s.is_empty()))
            {
                return Err(error("content arrived after completion"));
            }
            // Private reasoning fields are not a substitute for user-visible output.
            for key in ["content", "refusal"] {
                if let Some(text) = delta[key].as_str().filter(|s| !s.is_empty()) {
                    let mut chunk = self.chunk();
                    chunk.delta = Some(text.into());
                    self.queue.push_back(chunk);
                }
            }
            if let Some(calls) = delta.get("tool_calls").filter(|v| !v.is_null()) {
                let calls = calls
                    .as_array()
                    .ok_or_else(|| error("invalid tool-call delta"))?;
                for call in calls {
                    let index = call["index"]
                        .as_u64()
                        .filter(|n| *n < 1024)
                        .ok_or_else(|| error("invalid tool-call index"))?
                        as usize;
                    if self.tools.len() >= 128 && !self.tools.contains_key(&index) {
                        return Err(error("more than 128 tool calls"));
                    }
                    let tool = self.tools.entry(index).or_default();
                    if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
                        if !tool.id.is_empty() && tool.id != id {
                            return Err(error("tool-call ID changed"));
                        }
                        if tool.id.is_empty() {
                            self.tool_bytes += id.len();
                            tool.id = id.into();
                        }
                    }
                    for (key, target) in
                        [("name", &mut tool.name), ("arguments", &mut tool.arguments)]
                    {
                        if let Some(part) = call["function"].get(key).filter(|v| !v.is_null()) {
                            let part = part
                                .as_str()
                                .ok_or_else(|| error("function deltas must be strings"))?;
                            self.tool_bytes = self.tool_bytes.saturating_add(part.len());
                            target.push_str(part);
                        }
                    }
                    if self.tool_bytes > 2 * 1024 * 1024 {
                        return Err(error("tool arguments exceed 2 MiB limit"));
                    }
                }
            }
            if let Some(reason) = choice["finish_reason"].as_str() {
                if !matches!(reason, "stop" | "length" | "tool_calls" | "content_filter") {
                    return Err(error("unsupported or failed completion reason"));
                }
                if self.finished {
                    return Err(error("duplicate completion marker"));
                }
                if !self.tools.is_empty() && !matches!(reason, "tool_calls" | "stop") {
                    return Err(error("tool calls were truncated or filtered"));
                }
                let mut ready = Vec::new();
                for tool in self.tools.values() {
                    if tool.id.is_empty() || tool.name.is_empty() {
                        return Err(error("tool call lacks ID or name"));
                    }
                    let input: serde_json::Value = serde_json::from_str(&tool.arguments)
                        .map_err(|_| error("invalid tool arguments; no call will be dispatched"))?;
                    if !input.is_object() {
                        return Err(error("tool arguments must be an object"));
                    }
                    let mut chunk = self.chunk();
                    chunk.tool_use = Some(ContentPart::ToolUse {
                        id: tool.id.clone(),
                        name: tool.name.clone(),
                        input,
                        thought_signature: None,
                    });
                    ready.push(chunk);
                }
                self.tools.clear();
                self.queue.extend(ready);
                self.finished = true;
                let mut chunk = self.chunk();
                chunk.stop_reason = Some(reason.into());
                self.queue.push_back(chunk);
            }
        }
        Ok(())
    }
}

pub(crate) fn stream(
    response: reqwest::Response,
) -> BoxStream<'static, Result<StreamChunk, Temm1eError>> {
    let bytes = Box::pin(response.bytes_stream());
    let state = (
        bytes,
        SseDecoder::new(2 * 1024 * 1024),
        Decoder::default(),
        0usize,
        false,
    );
    Box::pin(futures::stream::unfold(
        state,
        |(mut bytes, mut sse, mut decoder, mut total, mut ended)| async move {
            loop {
                if let Some(chunk) = decoder.queue.pop_front() {
                    return Some((Ok(chunk), (bytes, sse, decoder, total, ended)));
                }
                if ended || decoder.done {
                    return None;
                }
                let result = match bytes.next().await {
                    Some(Ok(chunk)) => {
                        total = total.saturating_add(chunk.len());
                        if total > 64 * 1024 * 1024 {
                            Err(error("wire response exceeds 64 MiB limit"))
                        } else {
                            let mut result = Ok(());
                            for byte in chunk {
                                match sse.push(byte) {
                                    Ok(Some(event)) => {
                                        if let Err(e) = decoder.accept(&event.data) {
                                            result = Err(e);
                                            break;
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        result = Err(e);
                                        break;
                                    }
                                }
                            }
                            result
                        }
                    }
                    Some(Err(e)) => Err(Temm1eError::Provider(format!(
                        "Stream read failed: {}",
                        e.without_url()
                    ))),
                    None => {
                        ended = true;
                        sse.finish().and_then(|_| {
                            if decoder.finished {
                                Ok(())
                            } else {
                                Err(error("connection ended before completion"))
                            }
                        })
                    }
                };
                if let Err(e) = result {
                    decoder.queue.clear();
                    ended = true;
                    return Some((Err(e), (bytes, sse, decoder, total, ended)));
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interleaved_tools_and_final_usage_are_preserved_in_order() {
        let mut d = Decoder::default();
        d.accept(r#"{"id":"r","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"write","arguments":"{\"x\":"}},{"index":0,"id":"a","function":{"name":"read","arguments":"{}"}}]},"finish_reason":null}]}"#).unwrap();
        assert!(d.queue.is_empty());
        d.accept(r#"{"id":"r","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]},"finish_reason":"tool_calls"}]}"#).unwrap();
        d.accept(r#"{"id":"r","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":7,"prompt_tokens_details":{"cached_tokens":8,"cache_write_tokens":2}}}"#).unwrap();
        d.accept("[DONE]").unwrap();
        let chunks: Vec<_> = d.queue.into_iter().collect();
        assert!(matches!(&chunks[0].tool_use,Some(ContentPart::ToolUse{id,..}) if id=="a"));
        assert!(
            matches!(&chunks[1].tool_use,Some(ContentPart::ToolUse{id,input,..}) if id=="b" && input["x"]==2)
        );
        let usage = chunks[3].usage.as_ref().unwrap();
        assert_eq!(usage.input_tokens, 12);
        assert_eq!(usage.cache_read_tokens, Some(8));
        assert_eq!(usage.cache_write_tokens, Some(2));
    }
    #[test]
    fn invalid_arguments_and_premature_done_never_yield_executable_calls() {
        let mut d = Decoder::default();
        assert!(d.accept("[DONE]").is_err());
        d.accept(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"x","function":{"name":"write","arguments":"{bad"}}]},"finish_reason":null}]}"#).unwrap();
        assert!(d
            .accept(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#)
            .is_err());
        assert!(d.queue.iter().all(|c| c.tool_use.is_none()));
    }
}
