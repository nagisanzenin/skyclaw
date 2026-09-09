use async_trait::async_trait;
use futures::stream::BoxStream;
use std::{sync::Arc, time::Duration};
use temm1e_core::{
    types::{error::Temm1eError, message::*},
    Provider,
};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, MockProvider};
struct StreamingFixture {
    release: tokio::sync::Notify,
}
#[async_trait]
impl Provider for StreamingFixture {
    fn name(&self) -> &str {
        "fixture"
    }
    async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, Temm1eError> {
        Ok(MockProvider::with_text("auxiliary").response)
    }
    async fn complete_with_observer(
        &self,
        _: CompletionRequest,
        observer: temm1e_core::streaming::TextObserver,
    ) -> Result<CompletionResponse, Temm1eError> {
        observer("provisional");
        self.release.notified().await;
        Ok(MockProvider::with_text("Final response.").response)
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        Err(Temm1eError::Provider("unused".into()))
    }
    async fn health_check(&self) -> Result<bool, Temm1eError> {
        Ok(true)
    }
    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        Ok(vec![])
    }
}
#[tokio::test]
async fn foreground_observer_arrives_before_completion_with_stable_request_identity() {
    use temm1e_agent::agent_task_status::AgentTextEvent;
    let provider = Arc::new(StreamingFixture {
        release: tokio::sync::Notify::new(),
    });
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runtime = temm1e_agent::AgentRuntime::new(
        provider.clone(),
        Arc::new(MockMemory::new()),
        vec![],
        "fixture".into(),
        Some("Test".into()),
    )
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_text_observer(Arc::new(move |event| {
        tx.send(event).unwrap();
    }));
    let directory = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session.workspace_path = directory.path().to_owned();
    let task = tokio::spawn(async move {
        runtime
            .process_message(
                &make_inbound_msg("Hello"),
                &mut session,
                None,
                None,
                None,
                None,
                None,
            )
            .await
    });
    let begin = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let delta = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    match (begin, delta) {
        (AgentTextEvent::Begin { id: a }, AgentTextEvent::Delta { id: b, text }) => {
            assert_eq!(a, b);
            assert_eq!(text, "provisional");
        }
        _ => panic!("wrong lifecycle ordering"),
    }
    assert!(!task.is_finished());
    provider.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
