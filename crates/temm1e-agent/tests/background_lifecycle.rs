use async_trait::async_trait;
use futures::stream::BoxStream;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use temm1e_core::{
    types::{error::Temm1eError, message::*},
    Provider,
};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, MockProvider};

struct CuratorFixture {
    started: tokio::sync::Notify,
    dropped: Arc<AtomicBool>,
}
struct DropProbe(Arc<AtomicBool>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[async_trait]
impl Provider for CuratorFixture {
    fn name(&self) -> &str {
        "fixture"
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        if request
            .system
            .as_deref()
            .is_some_and(|s| s.contains("long-term-memory curator"))
        {
            let _probe = DropProbe(self.dropped.clone());
            self.started.notify_one();
            tokio::time::sleep(Duration::from_secs(60)).await;
            return Ok(MockProvider::with_text(r#"{"facts":[]}"#).response);
        }
        Ok(MockProvider::with_text("Acknowledged the project preference.").response)
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
async fn actual_curator_is_owned_after_foreground_reply_and_dropped_on_bounded_shutdown() {
    let dropped = Arc::new(AtomicBool::new(false));
    let provider = Arc::new(CuratorFixture {
        started: tokio::sync::Notify::new(),
        dropped: dropped.clone(),
    });
    let runtime = temm1e_agent::AgentRuntime::new(
        provider.clone(),
        Arc::new(MockMemory::new()),
        vec![],
        "fixture".into(),
        Some("Test".into()),
    )
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false);
    let directory = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session.workspace_path = directory.path().to_owned();
    tokio::time::timeout(Duration::from_secs(2),runtime.process_message(&make_inbound_msg("Our project uses Python standard library only. Please remember this durable project preference."),&mut session,None,None,None,None,None)).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), provider.started.notified())
        .await
        .unwrap();
    assert!(runtime.background_stats().pending > 0);
    assert!(!runtime.shutdown_background(Duration::from_millis(10)).await);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(runtime.background_stats().pending, 0);
    assert!(runtime.background_stats().cancelled > 0);
}
