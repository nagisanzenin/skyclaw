//! Native Messages stream lifecycle and cumulative usage, keyed by block index.
use crate::sse_transport::Protocol;
use futures::stream::BoxStream;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet, VecDeque};
use temm1e_core::{
    sse::SseEvent,
    types::{error::Temm1eError, message::*},
};
fn error(text: &str) -> Temm1eError {
    Temm1eError::Provider(format!("Anthropic stream: {text}"))
}
fn string<'a>(v: &'a Value, field: &str) -> Result<&'a str, Temm1eError> {
    v[field]
        .as_str()
        .ok_or_else(|| error("missing string field"))
}
fn index(v: &Value) -> Result<usize, Temm1eError> {
    v["index"]
        .as_u64()
        .filter(|n| *n < 1024)
        .map(|n| n as usize)
        .ok_or_else(|| error("invalid block index"))
}
enum Block {
    Text,
    Tool {
        id: String,
        name: String,
        initial: Value,
        json: String,
    },
    Thinking,
    Redacted,
}
#[derive(Default)]
struct Decoder {
    id: Option<String>,
    blocks: BTreeMap<usize, Block>,
    indices: HashSet<usize>,
    tool_ids: HashSet<String>,
    queue: VecDeque<StreamChunk>,
    raw_usage: Value,
    stop: Option<String>,
    done: bool,
    tool_bytes: usize,
    native_bytes: usize,
    output: BTreeMap<usize, Value>,
    route: String,
    model: String,
    context_fingerprint: String,
}
impl Decoder {
    fn chunk(&self) -> StreamChunk {
        StreamChunk {
            provider_state: None,
            response_id: self.id.clone(),
            delta: None,
            tool_use: None,
            usage: None,
            stop_reason: None,
        }
    }
    fn usage(&mut self, raw: &Value) -> Result<(), Temm1eError> {
        if raw.is_null() {
            return Ok(());
        }
        let raw = raw
            .as_object()
            .ok_or_else(|| error("invalid usage object"))?;
        if !self.raw_usage.is_object() {
            self.raw_usage = serde_json::json!({});
        }
        for key in [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ] {
            if let Some(value) = raw.get(key).filter(|v| !v.is_null()) {
                if value.as_u64().and_then(|n| u32::try_from(n).ok()).is_none() {
                    return Err(error("invalid usage counter"));
                }
                self.raw_usage[key] = value.clone();
            }
        }
        Ok(())
    }
    fn snapshot(&self) -> Result<Usage, Temm1eError> {
        let get = |key: &str| self.raw_usage[key].as_u64().map(|n| n as u32);
        let read = get("cache_read_input_tokens");
        let write = get("cache_creation_input_tokens");
        let input = get("input_tokens");
        let output = get("output_tokens");
        Ok(Usage {
            totals_reported: Some(input.is_some() && output.is_some()),
            input_tokens: input
                .unwrap_or(0)
                .checked_add(read.unwrap_or(0))
                .and_then(|n| n.checked_add(write.unwrap_or(0)))
                .ok_or_else(|| error("usage total overflow"))?,
            output_tokens: output.unwrap_or(0),
            cache_read_tokens: read,
            cache_write_tokens: write,
            cost_usd: 0.0,
        })
    }
}
impl Protocol for Decoder {
    fn pop(&mut self) -> Option<StreamChunk> {
        self.queue.pop_front()
    }
    fn done(&self) -> bool {
        self.done
    }
    fn finish(&self) -> Result<(), Temm1eError> {
        if self.done {
            Ok(())
        } else {
            Err(error("connection ended before message_stop"))
        }
    }
    fn accept(&mut self, event: SseEvent) -> Result<(), Temm1eError> {
        let value: Value =
            serde_json::from_str(&event.data).map_err(|_| error("invalid event JSON"))?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(&event.event);
        if event.event != "message" && event.event != kind {
            return Err(error("event name/type mismatch"));
        }
        if kind == "error" {
            return Err(error("provider returned an error event"));
        }
        if kind == "ping" {
            return Ok(());
        }
        if self.done {
            return Err(error("event after message_stop"));
        }
        if kind == "message_start" {
            if self.id.is_some() {
                return Err(error("duplicate message_start"));
            }
            let id = string(&value["message"], "id")?;
            if id.is_empty() {
                return Err(error("empty message ID"));
            }
            self.id = Some(id.into());
            self.usage(&value["message"]["usage"])?;
            return Ok(());
        }
        // Unknown event types remain forward compatible. Known lifecycle events require a start.
        if matches!(
            kind,
            "content_block_start"
                | "content_block_delta"
                | "content_block_stop"
                | "message_delta"
                | "message_stop"
        ) && self.id.is_none()
        {
            return Err(error("event before message_start"));
        }
        match kind {
            "content_block_start" => {
                let i = index(&value)?;
                if self.stop.is_some() || !self.indices.insert(i) {
                    return Err(error("duplicate or late content block"));
                }
                let b = &value["content_block"];
                self.native_bytes = self.native_bytes.saturating_add(b.to_string().len());
                if self.native_bytes > 16 * 1024 * 1024 {
                    return Err(error("native state exceeds 16 MiB"));
                }
                self.output.insert(i, b.clone());
                let block = match string(b, "type")? {
                    "text" => {
                        let text = string(b, "text")?;
                        if !text.is_empty() {
                            let mut c = self.chunk();
                            c.delta = Some(text.into());
                            self.queue.push_back(c);
                        }
                        Block::Text
                    }
                    "tool_use" => {
                        let id = string(b, "id")?;
                        let name = string(b, "name")?;
                        if id.is_empty()
                            || name.is_empty()
                            || !self.tool_ids.insert(id.into())
                            || self.tool_ids.len() > 128
                        {
                            return Err(error("invalid or duplicate tool identity"));
                        }
                        if !b["input"].is_object() {
                            return Err(error("initial tool input must be an object"));
                        }
                        self.tool_bytes = self
                            .tool_bytes
                            .saturating_add(id.len() + name.len() + b["input"].to_string().len());
                        Block::Tool {
                            id: id.into(),
                            name: name.into(),
                            initial: b["input"].clone(),
                            json: String::new(),
                        }
                    }
                    "server_tool_use" => {
                        return Err(error(
                            "server-side tools need a supported result/history adapter",
                        ))
                    }
                    "thinking" => {
                        string(b, "thinking")?;
                        Block::Thinking
                    }
                    "redacted_thinking" => {
                        string(b, "data")?;
                        Block::Redacted
                    }
                    _ => {
                        return Err(error(
                            "unsupported content block requires an explicit adapter",
                        ))
                    }
                };
                if self.tool_bytes > 2 * 1024 * 1024 {
                    return Err(error("tool input exceeds 2 MiB limit"));
                }
                self.blocks.insert(i, block);
            }
            "content_block_delta" => {
                let i = index(&value)?;
                let delta = &value["delta"];
                self.native_bytes = self.native_bytes.saturating_add(delta.to_string().len());
                if self.native_bytes > 16 * 1024 * 1024 {
                    return Err(error("native state exceeds 16 MiB"));
                }
                let native = self
                    .output
                    .get_mut(&i)
                    .ok_or_else(|| error("native output missing for delta"))?;
                let block = self
                    .blocks
                    .get_mut(&i)
                    .ok_or_else(|| error("delta for unopened block"))?;
                match (block, string(delta, "type")?) {
                    (Block::Text, "text_delta") => {
                        let text = string(delta, "text")?;
                        if let Value::String(value) = &mut native["text"] {
                            value.push_str(text);
                        } else {
                            return Err(error("invalid native text"));
                        }
                        let mut c = self.chunk();
                        c.delta = Some(text.into());
                        self.queue.push_back(c);
                    }
                    (Block::Tool { json, .. }, "input_json_delta") => {
                        let text = string(delta, "partial_json")?;
                        self.tool_bytes = self.tool_bytes.saturating_add(text.len());
                        if self.tool_bytes > 2 * 1024 * 1024 {
                            return Err(error("tool input exceeds 2 MiB limit"));
                        }
                        json.push_str(text);
                    }
                    (Block::Thinking, kind @ ("thinking_delta" | "signature_delta")) => {
                        let field = if kind == "thinking_delta" {
                            "thinking"
                        } else {
                            "signature"
                        };
                        if native.get(field).is_none() {
                            native[field] = Value::String(String::new());
                        }
                        if let Value::String(value) = &mut native[field] {
                            value.push_str(string(delta, field)?);
                        } else {
                            return Err(error("invalid native thinking field"));
                        }
                    }
                    (Block::Text, "citations_delta") => {
                        if !delta["citation"].is_object() {
                            return Err(error("invalid citation delta"));
                        }
                        if native.get("citations").is_none() {
                            native["citations"] = serde_json::json!([]);
                        }
                        native["citations"]
                            .as_array_mut()
                            .ok_or_else(|| error("invalid citations"))?
                            .push(delta["citation"].clone());
                    }
                    _ => return Err(error("unsupported or mismatched content delta")),
                }
            }
            "content_block_stop" => {
                let i = index(&value)?;
                let b = self
                    .blocks
                    .remove(&i)
                    .ok_or_else(|| error("stop for unopened block"))?;
                if let Block::Tool {
                    id,
                    name,
                    initial,
                    json,
                } = b
                {
                    let input = if json.is_empty() {
                        initial
                    } else {
                        serde_json::from_str(&json)
                            .map_err(|_| error("malformed tool arguments"))?
                    };
                    if !input.is_object() {
                        return Err(error("tool arguments must be an object"));
                    }
                    self.output
                        .get_mut(&i)
                        .ok_or_else(|| error("native tool output missing"))?["input"] = input;
                    // Dispatch normalized tool chunks only after message_stop.
                    let _ = (id, name);
                }
            }
            "message_delta" => {
                if !self.blocks.is_empty() {
                    return Err(error("message_delta before blocks closed"));
                }
                self.usage(&value["usage"])?;
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    if self.stop.as_deref().is_some_and(|old| old != reason) {
                        return Err(error("stop reason changed"));
                    }
                    if !self.tool_ids.is_empty() && reason != "tool_use" {
                        return Err(error("tool-bearing response did not finish with tool_use"));
                    }
                    self.stop = Some(reason.into());
                }
            }
            "message_stop" => {
                if !self.blocks.is_empty() || self.stop.is_none() {
                    return Err(error(
                        "message_stop without completed blocks and stop reason",
                    ));
                }
                let output: Vec<_> = self.output.values().cloned().collect();
                for part in crate::anthropic_native::normalize(&output)? {
                    if matches!(part, ContentPart::ToolUse { .. }) {
                        let mut chunk = self.chunk();
                        chunk.tool_use = Some(part);
                        self.queue.push_back(chunk);
                    }
                }
                let mut c = self.chunk();
                if !self.route.is_empty() {
                    c.provider_state = Some(ContentPart::ProviderState {
                        provider: self.route.clone(),
                        model: self.model.clone(),
                        response_id: self.id.clone().unwrap_or_default(),
                        context_fingerprint: Some(self.context_fingerprint.clone()),
                        output,
                    });
                }
                c.usage = Some(self.snapshot()?);
                c.stop_reason = self.stop.clone();
                self.queue.push_back(c);
                self.done = true;
            }
            _ => {}
        }
        Ok(())
    }
}
pub(crate) fn stream(
    response: reqwest::Response,
    route: String,
    model: String,
    context_fingerprint: String,
) -> BoxStream<'static, Result<StreamChunk, Temm1eError>> {
    crate::sse_transport::stream(
        response,
        Decoder {
            route,
            model,
            context_fingerprint,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn send(d: &mut Decoder, kind: &str, mut value: Value) -> Result<(), Temm1eError> {
        value["type"] = kind.into();
        d.accept(SseEvent {
            event: kind.into(),
            data: value.to_string(),
            id: String::new(),
        })
    }
    fn start(d: &mut Decoder) {
        send(d,"message_start",serde_json::json!({"message":{"id":"msg_1","usage":{"input_tokens":100,"output_tokens":1,"cache_read_input_tokens":80,"cache_creation_input_tokens":20}}})).unwrap();
    }
    #[test]
    fn streamed_thinking_and_redactions_survive_without_becoming_visible_deltas() {
        let mut decoder = Decoder {
            route: "anthropic|fixture".into(),
            model: "claude-opus-5".into(),
            context_fingerprint: "prefix".into(),
            ..Default::default()
        };
        start(&mut decoder);
        send(
            &mut decoder,
            "content_block_start",
            serde_json::json!({"index":0,"content_block":{"type":"thinking","thinking":""}}),
        )
        .unwrap();
        send(&mut decoder, "content_block_delta", serde_json::json!({"index":0,"delta":{"type":"thinking_delta","thinking":"private café"}})).unwrap();
        for signature in ["first-", "second"] {
            send(&mut decoder, "content_block_delta", serde_json::json!({"index":0,"delta":{"type":"signature_delta","signature":signature}})).unwrap();
        }
        send(
            &mut decoder,
            "content_block_stop",
            serde_json::json!({"index":0}),
        )
        .unwrap();
        send(&mut decoder, "content_block_start", serde_json::json!({"index":1,"content_block":{"type":"redacted_thinking","data":"opaque-data"}})).unwrap();
        send(
            &mut decoder,
            "content_block_stop",
            serde_json::json!({"index":1}),
        )
        .unwrap();
        assert!(decoder.queue.is_empty());
        send(&mut decoder, "content_block_start", serde_json::json!({"index":2,"content_block":{"type":"tool_use","id":"t1","name":"read","input":{}}})).unwrap();
        send(
            &mut decoder,
            "content_block_stop",
            serde_json::json!({"index":2}),
        )
        .unwrap();
        assert!(
            decoder.queue.is_empty(),
            "tools must wait for terminal completion"
        );
        send(
            &mut decoder,
            "message_delta",
            serde_json::json!({"delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}),
        )
        .unwrap();
        send(&mut decoder, "message_stop", serde_json::json!({})).unwrap();
        let chunks: Vec<_> = decoder.queue.into_iter().collect();
        assert!(chunks.iter().all(|chunk| chunk.delta.is_none()));
        assert!(matches!(&chunks[0].tool_use, Some(ContentPart::ToolUse { id, .. }) if id == "t1"));
        let Some(ContentPart::ProviderState {
            output,
            context_fingerprint,
            ..
        }) = &chunks[1].provider_state
        else {
            panic!("missing native state");
        };
        assert_eq!(output[0]["thinking"], "private café");
        assert_eq!(output[0]["signature"], "first-second");
        assert_eq!(output[1]["data"], "opaque-data");
        assert_eq!(context_fingerprint.as_deref(), Some("prefix"));
    }

    #[test]
    fn indexed_blocks_do_not_pop_other_tools_and_usage_is_cumulative() {
        let mut d = Decoder::default();
        start(&mut d);
        send(
            &mut d,
            "content_block_start",
            serde_json::json!({"index":0,"content_block":{"type":"text","text":"café "}}),
        )
        .unwrap();
        send(&mut d,"content_block_start",serde_json::json!({"index":1,"content_block":{"type":"tool_use","id":"a","name":"write","input":{}}})).unwrap();
        send(&mut d,"content_block_start",serde_json::json!({"index":2,"content_block":{"type":"tool_use","id":"b","name":"read","input":{}}})).unwrap();
        send(&mut d,"content_block_delta",serde_json::json!({"index":1,"delta":{"type":"input_json_delta","partial_json":"{\"x\":"}})).unwrap();
        send(&mut d, "content_block_stop", serde_json::json!({"index":0})).unwrap();
        assert_eq!(d.blocks.len(), 2);
        send(
            &mut d,
            "content_block_delta",
            serde_json::json!({"index":1,"delta":{"type":"input_json_delta","partial_json":"2}"}}),
        )
        .unwrap();
        send(&mut d, "content_block_stop", serde_json::json!({"index":1})).unwrap();
        send(&mut d, "content_block_stop", serde_json::json!({"index":2})).unwrap();
        send(
            &mut d,
            "message_delta",
            serde_json::json!({"delta":{},"usage":{"output_tokens":4}}),
        )
        .unwrap();
        send(&mut d,"message_delta",serde_json::json!({"delta":{"stop_reason":"tool_use"},"usage":{"input_tokens":110,"output_tokens":7}})).unwrap();
        assert!(!d.done());
        send(&mut d, "message_stop", serde_json::json!({})).unwrap();
        let chunks: Vec<_> = d.queue.into_iter().collect();
        assert_eq!(chunks[0].delta.as_deref(), Some("café "));
        assert!(
            matches!(&chunks[1].tool_use,Some(ContentPart::ToolUse{id,input,..}) if id=="a" && input["x"]==2)
        );
        assert!(
            matches!(&chunks[2].tool_use,Some(ContentPart::ToolUse{id,input,..}) if id=="b" && input==&serde_json::json!({}))
        );
        let u = chunks[3].usage.as_ref().unwrap();
        assert_eq!(u.input_tokens, 210);
        assert_eq!(u.output_tokens, 7);
        assert_eq!(u.cache_read_tokens, Some(80));
        assert_eq!(u.totals_reported, Some(true));
    }
    #[test]
    fn malformed_partial_and_error_events_do_not_finish() {
        let mut d = Decoder::default();
        assert!(send(&mut d, "message_stop", serde_json::json!({})).is_err());
        start(&mut d);
        send(&mut d,"content_block_start",serde_json::json!({"index":0,"content_block":{"type":"tool_use","id":"a","name":"write","input":{}}})).unwrap();
        send(&mut d,"content_block_delta",serde_json::json!({"index":0,"delta":{"type":"input_json_delta","partial_json":"{bad"}})).unwrap();
        assert!(send(&mut d, "content_block_stop", serde_json::json!({"index":0})).is_err());
        assert!(d.queue.iter().all(|c| c.tool_use.is_none()));
        assert!(d.finish().is_err());
        assert!(send(
            &mut d,
            "error",
            serde_json::json!({"error":{"type":"overloaded_error"}})
        )
        .is_err());
    }
    #[test]
    fn unknown_extensions_and_private_thinking_are_not_visible_text() {
        let mut d = Decoder::default();
        start(&mut d);
        send(&mut d, "future_event", serde_json::json!({})).unwrap();
        send(
            &mut d,
            "content_block_start",
            serde_json::json!({"index":0,"content_block":{"type":"thinking","thinking":""}}),
        )
        .unwrap();
        send(
            &mut d,
            "content_block_delta",
            serde_json::json!({"index":0,"delta":{"type":"thinking_delta","thinking":"private"}}),
        )
        .unwrap();
        send(&mut d, "content_block_stop", serde_json::json!({"index":0})).unwrap();
        assert!(d.queue.is_empty());
    }
}
