//! generateContent native parts and usage; no private thought text in display.
use serde_json::{json, Value};
use std::collections::HashSet;
use temm1e_core::types::{error::Temm1eError, message::*};

pub(crate) fn error(message: &str) -> Temm1eError {
    Temm1eError::Provider(format!("Gemini: {message}"))
}
pub(crate) fn route(base: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(base) else {
        return "gemini|invalid".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    format!("gemini|{}", url.as_str().trim_end_matches('/'))
}
pub(crate) fn normalize(raw: &[Value], response_id: &str) -> Result<Vec<ContentPart>, Temm1eError> {
    if raw.len() > 4096
        || serde_json::to_vec(raw)
            .map_err(|_| error("invalid parts"))?
            .len()
            > 16 * 1024 * 1024
    {
        return Err(error("native parts exceed limit"));
    }
    let mut result = Vec::new();
    let mut ids = HashSet::new();
    for (index, part) in raw.iter().enumerate() {
        let object = part.as_object().ok_or_else(|| error("invalid part"))?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "text" | "thought" | "thoughtSignature" | "functionCall" | "partMetadata"
            )
        }) {
            return Err(error(
                "unsupported native part requires an explicit adapter",
            ));
        }
        if part.get("thought").is_some_and(|v| !v.is_boolean())
            || part.get("thoughtSignature").is_some_and(|v| !v.is_string())
        {
            return Err(error("invalid thought metadata"));
        }
        if let Some(text) = part.get("text") {
            let text = text.as_str().ok_or_else(|| error("invalid text"))?;
            if part["thought"] != true {
                result.push(ContentPart::Text { text: text.into() });
            }
        }
        if let Some(call) = part.get("functionCall") {
            if part.get("text").is_some() || part["thought"] == true {
                return Err(error("ambiguous tool part"));
            }
            let name = call["name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| error("missing function name"))?;
            let fields = call
                .as_object()
                .ok_or_else(|| error("invalid function call"))?;
            if fields
                .keys()
                .any(|key| !matches!(key.as_str(), "id" | "name" | "args"))
            {
                return Err(error("unsupported function call fields"));
            }
            let arguments = call.get("args").cloned().unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return Err(error("function arguments must be an object"));
            }
            let id = match call.get("id") {
                Some(v) => v
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| error("invalid function ID"))?
                    .to_owned(),
                None => format!("gemini-{response_id}-{index}"),
            };
            if !ids.insert(id.clone()) || ids.len() > 128 {
                return Err(error("duplicate or excessive function calls"));
            }
            result.push(ContentPart::ToolUse {
                id,
                name: name.strip_prefix("default_api:").unwrap_or(name).into(),
                input: arguments,
                thought_signature: part["thoughtSignature"].as_str().map(str::to_owned),
            });
        }
        if part.get("text").is_none()
            && part.get("functionCall").is_none()
            && part.get("thoughtSignature").is_none()
        {
            return Err(error("empty native part"));
        }
    }
    Ok(result)
}
pub(crate) fn completion(
    raw: Vec<Value>,
    id: String,
    owner: &str,
    model: &str,
    stop: String,
    usage: Usage,
) -> Result<CompletionResponse, Temm1eError> {
    let mut content = normalize(&raw, &id)?;
    content.push(ContentPart::ProviderState {
        provider: owner.into(),
        model: model.into(),
        response_id: id.clone(),
        context_fingerprint: None,
        output: raw,
    });
    Ok(CompletionResponse {
        id,
        content,
        stop_reason: Some(stop),
        usage,
    })
}
pub(crate) fn replay(
    parts: &[ContentPart],
    owner: &str,
    model: &str,
) -> Result<Option<Vec<Value>>, Temm1eError> {
    let mut states = parts.iter().filter_map(|p| match p {
        ContentPart::ProviderState {
            provider,
            model: saved,
            response_id,
            output,
            ..
        } if provider == owner && saved == model => Some((response_id, output)),
        _ => None,
    });
    let Some((id, output)) = states.next() else {
        return Ok(None);
    };
    if states.next().is_some() {
        return Err(error("multiple native histories"));
    }
    let normalized = normalize(output, id)?;
    let calls = |parts: &[ContentPart]| {
        parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::ToolUse {
                    id, name, input, ..
                } => Some((id.clone(), name.clone(), input.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    if calls(parts) != calls(&normalized) {
        return Err(error("native and normalized function history disagree"));
    }
    let text = |parts: &[ContentPart]| {
        parts
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>()
    };
    let expected = text(&normalized);
    let current = text(parts);
    let mut output = output.clone();
    if current != expected {
        if let Some(extra) = current.strip_prefix(&expected) {
            output.push(json!({"text":extra}));
        } else {
            return Err(error(
                "native text history was replaced; explicit migration required",
            ));
        }
    }
    Ok(Some(output))
}
/// API totals include thoughts. Missing measurements remain unknown.
pub(crate) fn usage(value: &Value) -> Result<Usage, Temm1eError> {
    if value.is_null() {
        return Ok(Usage {
            totals_reported: Some(false),
            ..Usage::default()
        });
    }
    if !value.is_object() {
        return Err(error("invalid usage metadata"));
    }
    let count = |key: &str| -> Result<Option<u32>, Temm1eError> {
        value
            .get(key)
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| error("invalid usage counter"))
            })
            .transpose()
    };
    let input = count("promptTokenCount")?;
    let candidates = count("candidatesTokenCount")?;
    let thoughts = count("thoughtsTokenCount")?;
    let total = count("totalTokenCount")?;
    let cache = count("cachedContentTokenCount")?;
    if cache.zip(input).is_some_and(|(c, i)| c > i) {
        return Err(error("cache count exceeds input"));
    }
    let output = match (input, total) {
        (Some(i), Some(t)) => Some(t.checked_sub(i).ok_or_else(|| error("total below input"))?),
        _ => candidates
            .zip(thoughts)
            .map(|(c, t)| {
                c.checked_add(t)
                    .ok_or_else(|| error("output usage overflow"))
            })
            .transpose()?,
    };
    if output.zip(candidates).is_some_and(|(o, c)| o < c)
        || output.zip(thoughts).is_some_and(|(o, t)| o < t)
    {
        return Err(error("output below reported components"));
    }
    if let (Some(c), Some(t), Some(o)) = (candidates, thoughts, output) {
        if c.checked_add(t) != Some(o) {
            return Err(error("inconsistent output usage"));
        }
    }
    Ok(Usage {
        totals_reported: Some(input.is_some() && output.is_some()),
        input_tokens: input.unwrap_or(0),
        output_tokens: output.unwrap_or_else(|| candidates.unwrap_or(0)),
        cache_read_tokens: cache,
        cache_write_tokens: None,
        cost_usd: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn omitted_zero_argument_object_is_valid_but_partial_argument_protocol_is_not() {
        let parts = normalize(&[json!({"functionCall":{"name":"ping"}})], "r").unwrap();
        assert!(matches!(&parts[0], ContentPart::ToolUse {input,..} if input == &json!({})));
        assert!(normalize(
            &[json!({"functionCall":{"name":"ping","args":{},"partialArgs":[]}})],
            "r"
        )
        .is_err());
    }
    #[test]
    fn billing_includes_thoughts_and_preserves_unknown_totals() {
        let measured=usage(&json!({"promptTokenCount":100,"candidatesTokenCount":10,"thoughtsTokenCount":30,"totalTokenCount":140,"cachedContentTokenCount":25})).unwrap();
        assert_eq!(measured.output_tokens, 40);
        assert_eq!(measured.cache_read_tokens, Some(25));
        assert_eq!(measured.totals_reported, Some(true));
        assert_eq!(
            usage(&json!({"promptTokenCount":100,"candidatesTokenCount":10}))
                .unwrap()
                .totals_reported,
            Some(false)
        );
        for invalid in [
            json!({"promptTokenCount":100,"totalTokenCount":90}),
            json!({"promptTokenCount":100,"totalTokenCount":140,"candidatesTokenCount":10,"thoughtsTokenCount":31}),
            json!({"candidatesTokenCount":u32::MAX,"thoughtsTokenCount":1}),
            json!({"promptTokenCount":10,"cachedContentTokenCount":11}),
            json!({"promptTokenCount":-1}),
        ] {
            assert!(usage(&invalid).is_err());
        }
    }
    #[test]
    fn replay_rejects_changed_tools_and_does_not_cross_route_or_model() {
        let raw = vec![
            json!({"text":"private","thought":true,"thoughtSignature":"opaque"}),
            json!({"functionCall":{"name":"read_file","args":{"path":"a"}},"thoughtSignature":"signed-call"}),
        ];
        let response = completion(
            raw.clone(),
            "r".into(),
            "gemini|one",
            "m",
            "STOP".into(),
            Usage::default(),
        )
        .unwrap();
        assert!(!response
            .content
            .iter()
            .any(|p| matches!(p, ContentPart::Text { .. })));
        assert_eq!(
            replay(&response.content, "gemini|one", "m").unwrap(),
            Some(raw)
        );
        assert!(replay(&response.content, "gemini|two", "m")
            .unwrap()
            .is_none());
        assert!(replay(&response.content, "gemini|one", "other")
            .unwrap()
            .is_none());
        let mut changed = response.content;
        if let ContentPart::ToolUse { input, .. } = &mut changed[0] {
            *input = json!({"path":"b"});
        }
        assert!(replay(&changed, "gemini|one", "m").is_err());
    }
}
