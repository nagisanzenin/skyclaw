//! The actual runtime must bind observers to its current owner and model.
use std::sync::Arc;
use temm1e_agent::{
    consciousness::ConsciousnessConfig, consciousness_engine::ConsciousnessEngine, AgentRuntime,
    BudgetTracker,
};
use temm1e_test_utils::{
    make_inbound_msg, make_session, MockMemory, MockProvider, QueuedMockProvider,
};

#[tokio::test]
async fn observer_uses_current_resources_once_and_retains_trajectory() {
    let stale = Arc::new(MockProvider::with_text("obsolete observer"));
    let current = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("Track the required function signature."),
        QueuedMockProvider::text_response("First foreground reply."),
        QueuedMockProvider::text_response("POST_INSIGHT_SENTINEL keep the signature."),
        QueuedMockProvider::text_response("OK"),
        QueuedMockProvider::text_response("Second foreground reply."),
        QueuedMockProvider::text_response("OK"),
    ]));
    let owner = Arc::new(BudgetTracker::new(0.0));
    let runtime = AgentRuntime::new(
        current.clone(),
        Arc::new(MockMemory::new()),
        vec![],
        "current-model".into(),
        None,
    )
    .with_budget(owner.clone())
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_consciousness(ConsciousnessEngine::new(
        ConsciousnessConfig {
            enabled: true,
            ..Default::default()
        },
        stale.clone(),
        "obsolete-model".into(),
    ));
    let directory = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session.workspace_path = directory.path().into();
    for _ in 0..2 {
        runtime
            .process_message(
                &make_inbound_msg(
                    "In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String.",
                ),
                &mut session,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
    }
    runtime
        .shutdown_background(std::time::Duration::from_secs(1))
        .await;
    assert_eq!(
        stale.calls().await,
        0,
        "observer called its obsolete provider"
    );
    assert_eq!(current.calls().await, 6);
    assert_eq!(
        owner.snapshot().recorded_calls,
        6,
        "observer usage lost or charged twice"
    );
    let requests = current.captured_requests.lock().await;
    assert!(requests
        .iter()
        .all(|request| request.model == "current-model"));
    let second_pre = serde_json::to_string(&requests[3]).unwrap();
    assert!(
        second_pre.contains("POST_INSIGHT_SENTINEL"),
        "binding discarded trajectory"
    );
    assert!(second_pre.contains("Consciousness-T1"));
}

struct ObserverFailure {
    calls: std::sync::atomic::AtomicUsize,
    observers: std::sync::atomic::AtomicUsize,
    started: tokio::sync::Notify,
    pending: bool,
}
#[async_trait::async_trait]
impl temm1e_core::Provider for ObserverFailure {
    fn name(&self) -> &str {
        "fixture"
    }
    async fn complete(
        &self,
        request: temm1e_core::types::message::CompletionRequest,
    ) -> Result<
        temm1e_core::types::message::CompletionResponse,
        temm1e_core::types::error::Temm1eError,
    > {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if request
            .system
            .as_deref()
            .is_some_and(|text| text.contains("You are the consciousness layer"))
        {
            self.observers
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.started.notify_one();
            if self.pending {
                std::future::pending::<()>().await;
            }
            return Err(temm1e_core::types::error::Temm1eError::Provider(
                "fixture unavailable".into(),
            ));
        }
        Ok(MockProvider::with_text("Foreground reply.").response)
    }
    async fn stream(
        &self,
        _: temm1e_core::types::message::CompletionRequest,
    ) -> Result<
        futures::stream::BoxStream<
            '_,
            Result<
                temm1e_core::types::message::StreamChunk,
                temm1e_core::types::error::Temm1eError,
            >,
        >,
        temm1e_core::types::error::Temm1eError,
    > {
        Err(temm1e_core::types::error::Temm1eError::Provider(
            "unused".into(),
        ))
    }
    async fn health_check(&self) -> Result<bool, temm1e_core::types::error::Temm1eError> {
        Ok(true)
    }
    async fn list_models(&self) -> Result<Vec<String>, temm1e_core::types::error::Temm1eError> {
        Ok(vec![])
    }
}
fn failure_runtime(
    enabled: bool,
    pending: bool,
) -> (AgentRuntime, Arc<ObserverFailure>, Arc<BudgetTracker>) {
    let provider = Arc::new(ObserverFailure {
        calls: 0.into(),
        observers: 0.into(),
        started: tokio::sync::Notify::new(),
        pending,
    });
    let owner = Arc::new(BudgetTracker::new(0.0));
    let runtime = AgentRuntime::new(
        provider.clone(),
        Arc::new(MockMemory::new()),
        vec![],
        "current-model".into(),
        None,
    )
    .with_budget(owner.clone())
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_consciousness(ConsciousnessEngine::new(
        ConsciousnessConfig {
            enabled,
            ..Default::default()
        },
        Arc::new(MockProvider::with_text("obsolete observer")),
        "obsolete-model".into(),
    ));
    (runtime, provider, owner)
}
#[tokio::test]
async fn failed_observations_remain_metered_and_disabled_observer_makes_no_calls() {
    for enabled in [false, true] {
        let (runtime, provider, owner) = failure_runtime(enabled, false);
        let directory = tempfile::tempdir().unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().into();
        let (reply, _) = runtime
            .process_message(
                &make_inbound_msg(
                    "In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String.",
                ),
                &mut session,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        runtime
            .shutdown_background(std::time::Duration::from_secs(1))
            .await;
        assert_eq!(reply.text, "Foreground reply.");
        let expected = if enabled { 3 } else { 1 };
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            expected
        );
        assert_eq!(
            provider.observers.load(std::sync::atomic::Ordering::SeqCst),
            expected - 1
        );
        assert_eq!(owner.snapshot().recorded_calls, expected as u64);
        assert_eq!(owner.snapshot().unpriced_calls, expected as u64);
    }
}
#[tokio::test]
async fn dropped_observation_records_one_unknown_attempt_without_foreground_call() {
    let (runtime, provider, owner) = failure_runtime(true, true);
    let directory = tempfile::tempdir().unwrap();
    let mut session = make_session();
    session.workspace_path = directory.path().into();
    let message = make_inbound_msg(
        "In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String.",
    );
    let mut turn =
        Box::pin(runtime.process_message(&message, &mut session, None, None, None, None, None));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::select! {
            _ = provider.started.notified() => {},
            result = &mut turn => panic!("observer should be pending: {result:?}"),
        }
    })
    .await
    .expect("observer was never admitted");
    drop(turn);
    assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(owner.snapshot().recorded_calls, 1);
    assert_eq!(owner.snapshot().unpriced_calls, 1);
}
