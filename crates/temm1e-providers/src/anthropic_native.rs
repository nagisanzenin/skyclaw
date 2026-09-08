//! Native Messages output is replay state, not display text or new authority.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use temm1e_core::types::{error::Temm1eError, message::ContentPart};

fn error(message: &str) -> Temm1eError {
    Temm1eError::Provider(format!("Anthropic native state: {message}"))
}
fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str, Temm1eError> {
    value[field]
        .as_str()
        .ok_or_else(|| error("missing string field"))
}

pub(crate) fn route(base_url: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(base_url) else {
        return "anthropic|invalid-endpoint".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    format!("anthropic|{}", url.as_str().trim_end_matches('/'))
}

/// Object key order is canonicalized by serde_json; array/message order remains
/// significant. Only the prefix components bound by the provider are hashed.
pub(crate) fn context_fingerprint(body: &Value, messages: &[Value]) -> String {
    let prefix = json!({"system":body.get("system"),"tools":body.get("tools"),"messages":messages});
    hex::encode(Sha256::digest(prefix.to_string().as_bytes()))
}

pub(crate) fn normalize(output: &[Value]) -> Result<Vec<ContentPart>, Temm1eError> {
    if output.len() > 1024
        || serde_json::to_vec(output)
            .map_err(|_| error("invalid output"))?
            .len()
            > 16 * 1024 * 1024
    {
        return Err(error("output exceeds native replay limit"));
    }
    let mut result = Vec::new();
    let mut calls = HashSet::new();
    for block in output {
        match string(block, "type")? {
            "text" => result.push(ContentPart::Text {
                text: string(block, "text")?.into(),
            }),
            "tool_use" => {
                let id = string(block, "id")?;
                let name = string(block, "name")?;
                if id.is_empty()
                    || name.is_empty()
                    || !block["input"].is_object()
                    || !calls.insert(id)
                    || calls.len() > 128
                {
                    return Err(error("invalid or duplicate tool call"));
                }
                result.push(ContentPart::ToolUse {
                    id: id.into(),
                    name: name.into(),
                    input: block["input"].clone(),
                    thought_signature: None,
                });
            }
            "thinking" => {
                string(block, "thinking")?;
                if block.get("signature").is_some() {
                    string(block, "signature")?;
                }
            }
            "redacted_thinking" => {
                string(block, "data")?;
            }
            _ => {
                return Err(error(
                    "unsupported content block requires an explicit adapter",
                ))
            }
        }
    }
    Ok(result)
}

pub(crate) fn completion(
    id: String,
    output: Vec<Value>,
    owner: &str,
    model: &str,
    context_fingerprint: Option<String>,
) -> Result<Vec<ContentPart>, Temm1eError> {
    if id.is_empty() {
        return Err(error("missing response identity"));
    }
    let mut parts = normalize(&output)?;
    parts.push(ContentPart::ProviderState {
        provider: owner.into(),
        model: model.into(),
        response_id: id,
        context_fingerprint,
        output,
    });
    Ok(parts)
}

pub(crate) fn replay(
    parts: &[ContentPart],
    owner: &str,
    model: &str,
    expected_context: &str,
) -> Result<Option<Vec<Value>>, Temm1eError> {
    let mut native = parts.iter().filter_map(|part| match part {
        ContentPart::ProviderState {
            provider,
            model: original_model,
            output,
            context_fingerprint,
            ..
        } if provider == owner && original_model == model => Some((output, context_fingerprint)),
        _ => None,
    });
    let Some((output, stored_context)) = native.next() else {
        return Ok(None);
    };
    if native.next().is_some() {
        return Err(error("multiple native responses in one message"));
    }
    let normalized = normalize(output)?;
    let calls = |parts: &[ContentPart]| {
        parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolUse {
                    id, name, input, ..
                } => Some((id.clone(), name.clone(), input.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    if calls(parts) != calls(&normalized) {
        return Err(error("native and normalized tool history disagree"));
    }
    let text = |parts: &[ContentPart]| {
        parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut output = output.clone();
    if model == "claude-fable-5-1" && stored_context.as_deref() != Some(expected_context) {
        // Fable 5.1 binds thinking to system/tools/preceding messages. The
        // provider documents removing invalidated thinking while retaining
        // text/tools. Subsequent fingerprints naturally invalidate the suffix.
        output.retain(|block| {
            !matches!(
                block["type"].as_str(),
                Some("thinking" | "redacted_thinking")
            )
        });
    }
    let visible = text(parts);
    if visible != text(&normalized) && !visible.is_empty() {
        // Preserve original blocks byte-for-byte as JSON values and append a
        // distinct delivery correction. Never rewrite signed thinking blocks.
        output.push(json!({"type":"text","text":format!("Delivered reply after harness processing:\n{visible}")}));
    }
    Ok(Some(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_roundtrip_preserves_signatures_redactions_and_exact_tool_inputs() {
        let output = vec![
            json!({"type":"thinking","thinking":"","signature":"opaque-signature"}),
            json!({"type":"redacted_thinking","data":"opaque-data"}),
            json!({"type":"text","text":"Checking"}),
            json!({"type":"tool_use","id":"t1","name":"read","input":{"path":"a"}}),
        ];
        let owner = route("https://api.anthropic.com/");
        let mut parts = completion(
            "m1".into(),
            output.clone(),
            &owner,
            "claude-opus-5",
            Some("prefix".into()),
        )
        .unwrap();
        assert_eq!(parts.len(), 3); // Only text/tool plus hidden state.
        assert_eq!(
            replay(&parts, &owner, "claude-opus-5", "prefix").unwrap(),
            Some(output)
        );
        assert!(replay(
            &parts,
            &route("https://proxy.example"),
            "claude-opus-5",
            "prefix"
        )
        .unwrap()
        .is_none());
        assert!(replay(&parts, &owner, "different-model", "prefix")
            .unwrap()
            .is_none());
        parts.retain(|part| !matches!(part, ContentPart::ToolUse { .. }));
        assert!(replay(&parts, &owner, "claude-opus-5", "prefix").is_err());
    }
    #[test]
    fn invalid_native_calls_and_unknown_blocks_are_not_silently_dropped() {
        for output in [
            vec![json!({"type":"future_tool","input":{}})],
            vec![json!({"type":"tool_use","id":"a","name":"read","input":"not an object"})],
            vec![json!({"type":"tool_use","id":"a","name":"read","input":{}}); 2],
        ] {
            assert!(normalize(&output).is_err());
        }
    }
}
