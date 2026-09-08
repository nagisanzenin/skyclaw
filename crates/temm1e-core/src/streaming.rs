//! Assemble provider streams without treating provisional text or tool deltas
//! as a completed response. Usage events are cumulative snapshots, not additions.
use crate::types::{error::Temm1eError, message::*};
use futures::{stream::BoxStream, StreamExt};
use std::sync::Arc;
pub type TextObserver = Arc<dyn Fn(&str) + Send + Sync>;

pub async fn collect_completion(
    mut stream: BoxStream<'_, Result<StreamChunk, Temm1eError>>,
    observer: TextObserver,
) -> Result<CompletionResponse, Temm1eError> {
    let mut content = Vec::new();
    let mut text = String::new();
    let mut id = String::new();
    let mut stop_reason = None;
    let mut usage = Usage {
        totals_reported: Some(false),
        ..Usage::default()
    };
    let mut retained = 0usize;
    let mut provider_state = None;
    let mut tool_ids = std::collections::HashSet::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(response_id) = chunk.response_id.filter(|id| !id.is_empty()) {
            if !id.is_empty() && id != response_id {
                return Err(Temm1eError::Provider("Stream changed response ID".into()));
            }
            id = response_id;
        }
        if let Some(delta) = chunk.delta.filter(|s| !s.is_empty()) {
            retained = retained.saturating_add(delta.len());
            if retained > 32 * 1024 * 1024 {
                return Err(Temm1eError::Provider(
                    "Stream response exceeds 32 MiB retention limit".into(),
                ));
            }
            observer(&delta);
            text.push_str(&delta);
        }
        if let Some(tool) = chunk.tool_use {
            match &tool {
                ContentPart::ToolUse { id, .. }
                    if !id.is_empty() && tool_ids.insert(id.clone()) => {}
                _ => {
                    return Err(Temm1eError::Provider(
                        "Invalid or duplicate streamed tool call".into(),
                    ))
                }
            }
            if !text.is_empty() {
                content.push(ContentPart::Text {
                    text: std::mem::take(&mut text),
                });
            }
            retained = retained.saturating_add(
                serde_json::to_vec(&tool)
                    .map_err(|e| Temm1eError::Provider(e.to_string()))?
                    .len(),
            );
            if retained > 32 * 1024 * 1024 {
                return Err(Temm1eError::Provider(
                    "Stream response exceeds 32 MiB retention limit".into(),
                ));
            }
            content.push(tool);
        }
        if let Some(state) = chunk.provider_state {
            if !matches!(state, ContentPart::ProviderState { .. })
                || provider_state.is_some()
                || chunk.stop_reason.is_none()
            {
                return Err(Temm1eError::Provider(
                    "Invalid or provisional native response state".into(),
                ));
            }
            retained = retained.saturating_add(
                serde_json::to_vec(&state)
                    .map_err(|_| {
                        Temm1eError::Provider("Cannot serialize native response state".into())
                    })?
                    .len(),
            );
            if retained > 32 * 1024 * 1024 {
                return Err(Temm1eError::Provider(
                    "Native response exceeds retention limit".into(),
                ));
            }
            provider_state = Some(state);
        }
        if let Some(snapshot) = chunk.usage {
            usage = snapshot;
        }
        if chunk.stop_reason.is_some() {
            stop_reason = chunk.stop_reason;
        }
    }
    if stop_reason.is_none() {
        return Err(Temm1eError::Provider(
            "Stream ended without a completion marker; partial text is unconfirmed".into(),
        ));
    }
    if !text.is_empty() {
        content.push(ContentPart::Text { text });
    }
    if let Some(state) = provider_state {
        content.push(state);
    }
    Ok(CompletionResponse {
        id,
        content,
        stop_reason,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chunk(delta: Option<&str>, stop: Option<&str>, usage: Option<Usage>) -> StreamChunk {
        StreamChunk {
            provider_state: None,
            delta: delta.map(str::to_owned),
            stop_reason: stop.map(str::to_owned),
            usage,
            tool_use: None,
            response_id: Some("r".into()),
        }
    }
    #[tokio::test]
    async fn late_usage_is_retained_once_and_partial_text_requires_completion() {
        let seen = Arc::new(std::sync::Mutex::new(String::new()));
        let target = seen.clone();
        let observer: TextObserver = Arc::new(move |s| target.lock().unwrap().push_str(s));
        let parts = vec![
            Ok(chunk(Some("café "), None, None)),
            Ok(chunk(Some("🐈"), Some("stop"), None)),
            Ok(chunk(
                None,
                None,
                Some(Usage {
                    totals_reported: Some(true),
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Usage::default()
                }),
            )),
        ];
        let response = collect_completion(Box::pin(futures::stream::iter(parts)), observer.clone())
            .await
            .unwrap();
        assert_eq!(*seen.lock().unwrap(), "café 🐈");
        assert_eq!(response.usage.input_tokens, 10);
        assert_eq!(response.id, "r");
        assert!(collect_completion(
            Box::pin(futures::stream::iter(vec![Ok(chunk(
                Some("unfinished"),
                None,
                None
            ))])),
            observer
        )
        .await
        .is_err());
        let response = collect_completion(
            Box::pin(futures::stream::iter(vec![Ok(chunk(
                Some("text"),
                Some("stop"),
                None,
            ))])),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        assert_eq!(response.usage.totals_reported, Some(false));
    }
}
