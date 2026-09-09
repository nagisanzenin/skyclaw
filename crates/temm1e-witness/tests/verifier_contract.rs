use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::{Arc, Mutex};
use temm1e_core::{
    types::{error::Temm1eError, message::*},
    Provider,
};
use temm1e_witness::witness::{
    ProviderTier1Verifier, ProviderTier2Verifier, Tier1Verifier, Tier2Verifier,
};

struct Capture {
    requests: Mutex<Vec<CompletionRequest>>,
    response: CompletionResponse,
}
#[async_trait]
impl Provider for Capture {
    fn name(&self) -> &str {
        "fixture"
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        self.requests.lock().unwrap().push(request);
        Ok(self.response.clone())
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        unreachable!()
    }
    async fn health_check(&self) -> Result<bool, Temm1eError> {
        Ok(true)
    }
    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        Ok(vec![])
    }
}
fn capture(content: Vec<ContentPart>, stop: &str) -> Arc<Capture> {
    Arc::new(Capture {
        requests: Mutex::new(vec![]),
        response: CompletionResponse {
            id: "fixture".into(),
            content,
            stop_reason: Some(stop.into()),
            usage: Usage::default(),
        },
    })
}
fn text(value: &str) -> ContentPart {
    ContentPart::Text { text: value.into() }
}
const VALID: &str = r#"{"verdict":"INCONCLUSIVE","reason":"missing execution evidence"}"#;

#[tokio::test]
async fn adapters_preserve_model_bound_output_and_allow_abstention() {
    let provider = capture(vec![text(VALID)], "end_turn");
    let tier1 = ProviderTier1Verifier::new(provider.clone(), "selected-model");
    let tier2 = ProviderTier2Verifier::new(provider.clone(), "selected-model");
    assert_eq!(
        tier1
            .verify("goal", "rubric", "artifact bytes")
            .await
            .unwrap()
            .verdict,
        "inconclusive"
    );
    assert_eq!(
        tier2
            .audit("goal", "rubric", "artifact bytes")
            .await
            .unwrap()
            .verdict,
        "inconclusive"
    );
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for req in requests.iter() {
        assert_eq!(req.model, "selected-model");
        assert_eq!(req.max_tokens, Some(4096));
        assert!(req.tools.is_empty());
        assert_eq!(req.messages.len(), 1);
        let MessageContent::Text(prompt) = &req.messages[0].content else {
            panic!("non-text request")
        };
        assert!(prompt.contains("artifact bytes"));
        assert!(!prompt.contains("If you cannot find one, reply PASS"));
        assert!(req.system.as_ref().unwrap().contains("inconclusive"));
    }
}

#[tokio::test]
async fn invalid_input_never_reaches_either_adapter_provider() {
    let provider = capture(vec![text(VALID)], "end_turn");
    let tier1 = ProviderTier1Verifier::new(provider.clone(), "selected-model");
    let tier2 = ProviderTier2Verifier::new(provider.clone(), "selected-model");
    for (goal, rubric, evidence) in [
        ("".into(), "rubric".into(), "bytes".into()),
        ("g".repeat(16385), "rubric".into(), "bytes".into()),
        ("goal".into(), "r".repeat(8193), "bytes".into()),
        ("goal".into(), "rubric".into(), "e".repeat(65537)),
    ] {
        assert!(tier1.verify(&goal, &rubric, &evidence).await.is_err());
        assert!(tier2.audit(&goal, &rubric, &evidence).await.is_err());
    }
    assert!(provider.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn neither_adapter_accepts_truncated_tool_or_oversized_completion() {
    for (content, stop) in [
        (vec![text(VALID)], "length"),
        (vec![text(VALID)], "max_tokens"),
        (
            vec![
                text(VALID),
                ContentPart::ToolUse {
                    id: "t".into(),
                    name: "shell".into(),
                    input: serde_json::json!({}),
                    thought_signature: None,
                },
            ],
            "tool_use",
        ),
        (vec![text(&" ".repeat(16384)), text(VALID)], "end_turn"),
        (
            vec![
                text("The following is an example, not my verdict: "),
                text(VALID),
            ],
            "end_turn",
        ),
    ] {
        let provider = capture(content, stop);
        assert!(ProviderTier1Verifier::new(provider.clone(), "model")
            .verify("goal", "rubric", "bytes")
            .await
            .is_err());
        assert!(ProviderTier2Verifier::new(provider.clone(), "model")
            .audit("goal", "rubric", "bytes")
            .await
            .is_err());
        assert_eq!(provider.requests.lock().unwrap().len(), 2);
    }
}
