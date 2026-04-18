use async_trait::async_trait;
use std::sync::Arc;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::message::{ChatMessage, CompletionRequest, MessageContent, Role};

use crate::store::MonitorHistoryEntry;
use crate::tracing_ext;
use crate::types::{Interpretation, ScheduleReview, Urgency};

/// Trait for making LLM calls — abstracts over Provider for testability.
#[async_trait]
pub trait LlmCaller: Send + Sync {
    async fn call(&self, system: Option<&str>, prompt: &str) -> Result<String, Temm1eError>;
}

/// Production implementation using temm1e Provider trait.
pub struct ProviderCaller {
    provider: Arc<dyn temm1e_core::traits::Provider>,
    model: String,
}

impl ProviderCaller {
    pub fn new(provider: Arc<dyn temm1e_core::traits::Provider>, model: String) -> Self {
        Self { provider, model }
    }
}

#[async_trait]
impl LlmCaller for ProviderCaller {
    async fn call(&self, system: Option<&str>, prompt: &str) -> Result<String, Temm1eError> {
        let request = CompletionRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text(prompt.to_string()),
            }],
            tools: vec![],
            max_tokens: None,
            temperature: Some(0.2),
            system: system.map(String::from),
            system_volatile: None,
        };

        // Timeout: 300s (5 min) for LLM calls. Reasoning models (o1, o3, DeepSeek R1)
        // can take several minutes for complex tasks. Short timeouts would cut them off
        // mid-reasoning, producing incomplete/broken output.
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            self.provider.complete(request),
        )
        .await
        .map_err(|_| Temm1eError::Provider("Perpetuum LLM call timed out (300s)".to_string()))??;
        let text = response
            .content
            .iter()
            .filter_map(|p| match p {
                temm1e_core::types::message::ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        Ok(text)
    }
}

/// LLM-powered check interpretation and schedule review.
pub struct Cognitive {
    caller: Arc<dyn LlmCaller>,
}

impl Cognitive {
    pub fn new(caller: Arc<dyn LlmCaller>) -> Self {
        Self { caller }
    }

    pub fn from_provider(provider: Arc<dyn temm1e_core::traits::Provider>, model: String) -> Self {
        Self {
            caller: Arc::new(ProviderCaller::new(provider, model)),
        }
    }

    /// Layer 2: Interpret monitor check results against user intent.
    pub async fn interpret(
        &self,
        concern_id: &str,
        monitor_name: &str,
        user_intent: &str,
        new_content: &str,
        last_content: Option<&str>,
    ) -> Result<Interpretation, Temm1eError> {
        let prev_section = last_content
            .map(|c| format!("\nPrevious content:\n{c}\n"))
            .unwrap_or_default();

        let prompt = format!(
            "You are evaluating monitor results for the user.\n\
             Monitor: \"{monitor_name}\"\n\
             User's intent: \"{user_intent}\"\n\
             \n\
             New content found:\n{new_content}\n\
             {prev_section}\n\
             Respond ONLY in JSON (no markdown, no explanation):\n\
             {{\"relevant\":true/false,\"urgency\":\"low\"|\"medium\"|\"high\"|\"critical\",\"notify\":true/false,\"summary\":\"concise text or null\"}}"
        );

        let text = self.caller.call(None, &prompt).await?;
        let interpretation = parse_interpretation(&text).unwrap_or(Interpretation {
            relevant: true,
            urgency: Urgency::Medium,
            notify: true,
            summary: Some(truncate(new_content, 200)),
        });

        tracing_ext::trace_cognitive_eval(
            concern_id,
            "interpret",
            &format!(
                "relevant={} urgency={:?} notify={}",
                interpretation.relevant, interpretation.urgency, interpretation.notify
            ),
        );

        Ok(interpretation)
    }

    /// Layer 3: Review monitoring schedule and recommend adjustments.
    pub async fn review_schedule(
        &self,
        concern_id: &str,
        monitor_name: &str,
        user_intent: &str,
        history: &[MonitorHistoryEntry],
        current_interval_secs: u64,
        temporal_context: &str,
    ) -> Result<ScheduleReview, Temm1eError> {
        let history_text = format_history(history);
        let interval_str = format_interval(current_interval_secs);

        let prompt = format!(
            "You are reviewing a monitoring schedule.\n\
             Monitor: \"{monitor_name}\"\n\
             User's intent: \"{user_intent}\"\n\
             Current interval: {interval_str}\n\
             \n\
             Recent check history ({} checks):\n{history_text}\n\
             \n\
             {temporal_context}\n\
             \n\
             Respond ONLY in JSON:\n\
             {{\"action\":\"keep\"|\"adjust\",\"new_interval_secs\":number_or_null,\"reasoning\":\"brief\",\"user_recommendation\":\"message or null\"}}",
            history.len()
        );

        let text = self.caller.call(None, &prompt).await?;
        let review = parse_schedule_review(&text).unwrap_or(ScheduleReview {
            action: "keep".to_string(),
            new_interval_secs: None,
            reasoning: "Failed to parse LLM response, keeping current schedule".to_string(),
            user_recommendation: None,
        });

        tracing_ext::trace_cognitive_eval(
            concern_id,
            "schedule_review",
            &format!(
                "action={} new_interval={:?}",
                review.action, review.new_interval_secs
            ),
        );

        Ok(review)
    }
}

fn parse_interpretation(text: &str) -> Option<Interpretation> {
    // Try to extract JSON from the response (may have surrounding text)
    let json_str = extract_json(text)?;
    let v: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    Some(Interpretation {
        relevant: v.get("relevant")?.as_bool()?,
        urgency: match v.get("urgency")?.as_str()? {
            "low" => Urgency::Low,
            "high" => Urgency::High,
            "critical" => Urgency::Critical,
            _ => Urgency::Medium,
        },
        notify: v.get("notify")?.as_bool()?,
        summary: v.get("summary").and_then(|s| s.as_str()).map(String::from),
    })
}

fn parse_schedule_review(text: &str) -> Option<ScheduleReview> {
    let json_str = extract_json(text)?;
    let v: serde_json::Value = serde_json::from_str(&json_str).ok()?;

    Some(ScheduleReview {
        action: v.get("action")?.as_str()?.to_string(),
        new_interval_secs: v.get("new_interval_secs").and_then(|n| n.as_u64()),
        reasoning: v
            .get("reasoning")
            .and_then(|s| s.as_str())
            .unwrap_or("no reasoning")
            .to_string(),
        user_recommendation: v
            .get("user_recommendation")
            .and_then(|s| s.as_str())
            .map(String::from),
    })
}

/// Extract the first JSON object from text (public for volition module).
pub fn extract_json_from_text(text: &str) -> Option<String> {
    extract_json(text)
}

/// Extract the first JSON object from text (handles markdown code blocks and surrounding text).
fn extract_json(text: &str) -> Option<String> {
    // Try direct parse first
    if let Ok(_v) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        return Some(text.trim().to_string());
    }
    // Look for JSON between braces
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end > start {
        Some(text[start..=end].to_string())
    } else {
        None
    }
}

fn format_history(history: &[MonitorHistoryEntry]) -> String {
    if history.is_empty() {
        return "No previous checks".to_string();
    }
    history
        .iter()
        .take(10)
        .map(|h| {
            let change = if h.change_detected { "CHANGED" } else { "same" };
            let notified = if h.notified { " [notified]" } else { "" };
            let preview = h.raw_content_preview.as_deref().unwrap_or("[no preview]");
            format!(
                "- {} {} {}{}",
                h.checked_at,
                change,
                truncate(preview, 80),
                notified
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_interval(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .take_while(|(i, _)| *i < max)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(max);
        format!("{}...", &s[..boundary])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_direct() {
        let text = r#"{"relevant":true,"urgency":"high","notify":true,"summary":"test"}"#;
        assert!(extract_json(text).is_some());
    }

    #[test]
    fn extract_json_with_surrounding_text() {
        let text = "Here is the analysis:\n```json\n{\"relevant\":true}\n```\nDone.";
        let json = extract_json(text).unwrap();
        assert!(json.contains("relevant"));
    }

    #[test]
    fn parse_interpretation_valid() {
        let text = r#"{"relevant":true,"urgency":"high","notify":true,"summary":"important post"}"#;
        let interp = parse_interpretation(text).unwrap();
        assert!(interp.relevant);
        assert!(matches!(interp.urgency, Urgency::High));
        assert!(interp.notify);
        assert_eq!(interp.summary.unwrap(), "important post");
    }

    #[test]
    fn parse_interpretation_invalid_returns_none() {
        assert!(parse_interpretation("not json at all").is_none());
    }

    #[test]
    fn parse_schedule_review_valid() {
        let text = r#"{"action":"adjust","new_interval_secs":600,"reasoning":"quiet at night","user_recommendation":null}"#;
        let review = parse_schedule_review(text).unwrap();
        assert_eq!(review.action, "adjust");
        assert_eq!(review.new_interval_secs, Some(600));
    }

    #[test]
    fn format_interval_display() {
        assert_eq!(format_interval(30), "30s");
        assert_eq!(format_interval(300), "5m");
        assert_eq!(format_interval(3900), "1h5m");
    }

    #[test]
    fn truncate_safe_utf8() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world this is long", 10), "hello worl...");
        // UTF-8 safety: Vietnamese text
        let viet = "Xin ch\u{00e0}o th\u{1ebf} gi\u{1edb}i";
        let t = truncate(viet, 8);
        assert!(t.len() <= 15); // truncated + "..."
    }
}
