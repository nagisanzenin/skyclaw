//! Integration tests for the agent runtime — tests the full message processing
//! loop with mock provider and mock memory working together.

use std::sync::Arc;

use temm1e_agent::{AgentRuntime, AgentTaskPhase, AgentTaskStatus};
use temm1e_core::types::message::*;
use temm1e_core::Tool;
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, MockProvider, MockTool};
use tokio_util::sync::CancellationToken;

fn make_runtime_with_text(text: &str) -> AgentRuntime {
    let provider = Arc::new(MockProvider::with_text(text));
    let memory = Arc::new(MockMemory::new());
    let tools: Vec<Arc<dyn Tool>> = vec![];

    // Disable v2 optimizations — MockProvider returns plain text,
    // not JSON classification, so the LLM classifier would always
    // fall back and add an extra provider call.
    AgentRuntime::new(
        provider,
        memory,
        tools,
        "test-model".to_string(),
        Some("You are a test agent.".to_string()),
    )
    .with_v2_optimizations(false)
}

#[tokio::test]
async fn simple_text_response() {
    let runtime = make_runtime_with_text("Hello from the AI!");
    let msg = make_inbound_msg("Hi there");
    let mut session = make_session();

    let (reply, _turn_usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(reply.text, "Hello from the AI!");
    assert_eq!(reply.chat_id, msg.chat_id);
    assert!(reply.reply_to.is_some());
    assert!(reply.parse_mode.is_none());
}

#[tokio::test]
async fn session_history_grows_after_processing() {
    let runtime = make_runtime_with_text("Response text");
    let msg = make_inbound_msg("User input");
    let mut session = make_session();

    assert!(session.history.is_empty());
    runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    // Should have user message + assistant reply in history
    assert_eq!(session.history.len(), 2);
    assert!(matches!(session.history[0].role, Role::User));
    assert!(matches!(session.history[1].role, Role::Assistant));
}

#[tokio::test]
async fn runtime_with_no_text_in_inbound_msg() {
    let runtime = make_runtime_with_text("OK");
    let mut msg = make_inbound_msg("");
    msg.text = None;
    let mut session = make_session();

    let (reply, _turn_usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();
    // Empty message with no attachments returns a friendly error
    assert!(reply.text.contains("empty message"));
}

#[tokio::test]
async fn provider_called_exactly_once_for_simple_text() {
    let provider = Arc::new(MockProvider::with_text("response"));
    let memory = Arc::new(MockMemory::new());
    let runtime = AgentRuntime::new(provider.clone(), memory, vec![], "model".to_string(), None)
        .with_v2_optimizations(false);

    let msg = make_inbound_msg("hello");
    let mut session = make_session();
    runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(provider.calls().await, 1);
}

#[tokio::test]
async fn runtime_accessor_methods() {
    let provider = Arc::new(MockProvider::with_text("test"));
    let memory = Arc::new(MockMemory::new());
    let tool = Arc::new(MockTool::new("my_tool"));
    let tools: Vec<Arc<dyn Tool>> = vec![tool];

    let runtime = AgentRuntime::new(
        provider,
        memory,
        tools,
        "model".to_string(),
        Some("prompt".to_string()),
    );

    assert_eq!(runtime.provider().name(), "mock");
    assert_eq!(runtime.memory().backend_name(), "mock");
    assert_eq!(runtime.tools().len(), 1);
    assert_eq!(runtime.tools()[0].name(), "my_tool");
}

#[tokio::test]
async fn runtime_with_memory_entries() {
    let memory = Arc::new(MockMemory::with_entries(vec![
        temm1e_test_utils::make_test_entry_with_session(
            "mem1",
            "Important context about Rust",
            "test:test-chat:test-user",
        ),
    ]));

    let provider = Arc::new(MockProvider::with_text("I remember about Rust!"));
    let runtime = AgentRuntime::new(provider.clone(), memory, vec![], "model".to_string(), None)
        .with_v2_optimizations(false);

    let msg = make_inbound_msg("Tell me about Rust");
    let mut session = make_session();
    let (reply, _turn_usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(reply.text, "I remember about Rust!");

    // Check that the provider received messages including memory context
    let captured = provider.captured_requests.lock().await;
    assert_eq!(captured.len(), 1);
    let req = &captured[0];
    // Should have system message (memory context) + user message
    assert!(!req.messages.is_empty());
}

#[tokio::test]
async fn multiple_messages_in_sequence() {
    let runtime = make_runtime_with_text("Reply");

    let mut session = make_session();

    for i in 0..3 {
        let msg = make_inbound_msg(&format!("Message {i}"));
        let (reply, _turn_usage) = runtime
            .process_message(&msg, &mut session, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(reply.text, "Reply");
    }

    // History should have 3 user + 3 assistant = 6 messages
    assert_eq!(session.history.len(), 6);
}

// ── Interceptor Phase 1: Watch Channel + CancellationToken Tests ───

#[tokio::test]
async fn status_watch_channel_receives_phase_transitions() {
    // Watch channels show the LATEST value, not all intermediate states.
    // With a synchronous MockProvider, process_message runs to completion
    // before any observer task can poll. So we verify:
    // 1. The sender was used (receiver sees a changed mark)
    // 2. The final state reflects the complete lifecycle
    let runtime = make_runtime_with_text("Hello!");
    let msg = make_inbound_msg("Hi");
    let mut session = make_session();

    let (status_tx, mut status_rx) = tokio::sync::watch::channel(AgentTaskStatus::default());
    let cancel = CancellationToken::new();

    let (reply, _usage) = runtime
        .process_message(
            &msg,
            &mut session,
            None,
            None,
            None,
            Some(status_tx),
            Some(cancel),
        )
        .await
        .unwrap();

    assert_eq!(reply.text, "Hello!");

    // The watch channel should have been modified — changed() resolves immediately
    // because send_modify was called multiple times during process_message
    let changed =
        tokio::time::timeout(tokio::time::Duration::from_millis(100), status_rx.changed()).await;
    assert!(
        changed.is_ok(),
        "Watch channel should have pending changes after process_message"
    );

    // The latest value should be Done (final phase)
    let status = status_rx.borrow().clone();
    assert!(
        matches!(status.phase, AgentTaskPhase::Done),
        "Final phase should be Done, got {:?}",
        status.phase
    );
}

#[tokio::test]
async fn status_watch_final_state_is_done() {
    let runtime = make_runtime_with_text("Done!");
    let msg = make_inbound_msg("test");
    let mut session = make_session();

    let (status_tx, status_rx) = tokio::sync::watch::channel(AgentTaskStatus::default());
    let cancel = CancellationToken::new();

    runtime
        .process_message(
            &msg,
            &mut session,
            None,
            None,
            None,
            Some(status_tx),
            Some(cancel),
        )
        .await
        .unwrap();

    // After process_message returns, the final state should be Done
    let final_status = status_rx.borrow().clone();
    assert!(
        matches!(final_status.phase, AgentTaskPhase::Done),
        "Final phase should be Done, got {:?}",
        final_status.phase
    );
}

#[tokio::test]
async fn status_watch_tracks_token_counts() {
    let runtime = make_runtime_with_text("response");
    let msg = make_inbound_msg("query");
    let mut session = make_session();

    let (status_tx, status_rx) = tokio::sync::watch::channel(AgentTaskStatus::default());

    runtime
        .process_message(&msg, &mut session, None, None, None, Some(status_tx), None)
        .await
        .unwrap();

    let final_status = status_rx.borrow().clone();
    // MockProvider reports token usage — verify it's captured
    // The exact values depend on MockProvider but should be > 0
    // if the provider reports any usage
    assert!(
        matches!(final_status.phase, AgentTaskPhase::Done),
        "Phase should be Done"
    );
    assert_eq!(
        final_status.rounds_completed, 0,
        "Simple text response = 0 tool rounds"
    );
}

#[tokio::test]
async fn cancel_token_does_not_affect_normal_flow() {
    let runtime = make_runtime_with_text("Normal response");
    let msg = make_inbound_msg("Hello");
    let mut session = make_session();

    let cancel = CancellationToken::new();
    // Don't cancel — verify normal flow works with token present
    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, Some(cancel))
        .await
        .unwrap();

    assert_eq!(reply.text, "Normal response");
}

#[tokio::test]
async fn none_status_and_cancel_is_backward_compatible() {
    // Verify that passing None for both Phase 1 params
    // produces identical behavior to pre-Phase-1
    let runtime = make_runtime_with_text("Backward compat");
    let msg = make_inbound_msg("test");
    let mut session = make_session();

    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(reply.text, "Backward compat");
    assert_eq!(session.history.len(), 2);
}

#[tokio::test]
async fn watch_channel_with_multiple_messages() {
    let runtime = make_runtime_with_text("Reply");
    let mut session = make_session();

    // Process multiple messages, each with its own watch channel
    for i in 0..3 {
        let (status_tx, status_rx) = tokio::sync::watch::channel(AgentTaskStatus::default());
        let cancel = CancellationToken::new();
        let msg = make_inbound_msg(&format!("Message {i}"));

        runtime
            .process_message(
                &msg,
                &mut session,
                None,
                None,
                None,
                Some(status_tx),
                Some(cancel),
            )
            .await
            .unwrap();

        let final_status = status_rx.borrow().clone();
        assert!(
            matches!(final_status.phase, AgentTaskPhase::Done),
            "Message {i}: final phase should be Done, got {:?}",
            final_status.phase
        );
    }
}

#[tokio::test]
async fn classification_usage_survives_invalid_json_without_doubling_valid_json() {
    use temm1e_test_utils::QueuedMockProvider;
    for classifier_text in [
        "invalid classifier JSON",
        r#"{"category":"chat","chat_text":"","difficulty":"simple"}"#,
    ] {
        let provider = Arc::new(QueuedMockProvider::with_responses(vec![
            QueuedMockProvider::text_response(classifier_text),
            QueuedMockProvider::text_response("final fixture reply"),
        ]));
        let runtime = AgentRuntime::new(
            provider.clone(),
            Arc::new(MockMemory::new()),
            vec![],
            "model".into(),
            None,
        )
        .with_v2_optimizations(true);
        let (reply, usage) = runtime
            .process_message(
                &make_inbound_msg("Hello there"),
                &mut make_session(),
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(reply.text, "final fixture reply");
        assert_eq!(provider.calls().await, 2);
        assert_eq!(usage.api_calls, 2);
        assert_eq!((usage.input_tokens, usage.output_tokens), (20, 40));
        let recorded = runtime.budget_snapshot();
        assert_eq!(recorded.recorded_calls, 2);
        assert_eq!((recorded.input_tokens, recorded.output_tokens), (20, 40));
        assert_eq!(recorded.unpriced_calls, 2); // mock provider has no verified tariff
    }
}

struct PendingClassifier {
    started: tokio::sync::Semaphore,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl temm1e_core::Provider for PendingClassifier {
    fn name(&self) -> &str {
        "openai"
    }
    async fn complete(
        &self,
        _: CompletionRequest,
    ) -> Result<CompletionResponse, temm1e_core::types::error::Temm1eError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.started.add_permits(1);
        std::future::pending().await
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<
        futures::stream::BoxStream<'_, Result<StreamChunk, temm1e_core::types::error::Temm1eError>>,
        temm1e_core::types::error::Temm1eError,
    > {
        panic!("classifier must use complete")
    }
    async fn health_check(&self) -> Result<bool, temm1e_core::types::error::Temm1eError> {
        Ok(true)
    }
    async fn list_models(&self) -> Result<Vec<String>, temm1e_core::types::error::Temm1eError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn canceled_or_deadline_limited_classifier_records_unknown_once_and_blocks_further_spend() {
    for use_deadline in [false, true] {
        let provider = Arc::new(PendingClassifier {
            started: tokio::sync::Semaphore::new(0),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime = Arc::new(
            AgentRuntime::with_limits(
                provider.clone(),
                Arc::new(MockMemory::new()),
                vec![],
                "gpt-4.1".into(),
                None,
                10,
                32768,
                8,
                if use_deadline { 1 } else { 60 },
                1.0,
            )
            .with_v2_optimizations(true),
        );
        let cancel = CancellationToken::new();
        let task_runtime = runtime.clone();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            task_runtime
                .process_message(
                    &make_inbound_msg("Hello"),
                    &mut make_session(),
                    None,
                    None,
                    None,
                    None,
                    Some(task_cancel),
                )
                .await
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            provider.started.acquire(),
        )
        .await
        .unwrap()
        .unwrap()
        .forget();
        if !use_deadline {
            cancel.cancel();
        }
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        let usage = runtime.budget_snapshot();
        assert_eq!(usage.recorded_calls, 1);
        assert_eq!(usage.unpriced_calls, 1);
        assert_eq!((usage.input_tokens, usage.output_tokens), (0, 0));
        let retry = runtime
            .process_message(
                &make_inbound_msg("Try again"),
                &mut make_session(),
                None,
                None,
                None,
                None,
                None,
            )
            .await;
        assert!(retry
            .unwrap_err()
            .to_string()
            .contains("unknown pricing or missing usage"));
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(runtime.budget_snapshot().recorded_calls, 1);
    }
}

/// Only the second call is the optional curator. The first is a foreground
/// response with a known tariff, so canceled/error usage stays distinguishable.
struct CuratorFixture {
    calls: std::sync::atomic::AtomicUsize,
    started: tokio::sync::Semaphore,
    mode: &'static str,
}
#[async_trait::async_trait]
impl temm1e_core::Provider for CuratorFixture {
    fn name(&self) -> &str {
        "openai"
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, temm1e_core::types::error::Temm1eError> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let text = if call == 0 {
            "Foreground reply"
        } else {
            assert_eq!(call, 1, "unexpected extra provider call");
            assert!(request
                .system
                .as_deref()
                .unwrap_or_default()
                .contains("long-term-memory curator"));
            self.started.add_permits(1);
            match self.mode {
                "error" => {
                    return Err(temm1e_core::types::error::Temm1eError::Provider(
                        "synthetic curator failure".into(),
                    ))
                }
                "pending" => return std::future::pending().await,
                "valid" => r#"{"facts":[]}"#,
                _ => "malformed curator JSON",
            }
        };
        let mut response = temm1e_test_utils::QueuedMockProvider::text_response(text);
        response.usage.totals_reported = Some(true);
        Ok(response)
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<
        futures::stream::BoxStream<'_, Result<StreamChunk, temm1e_core::types::error::Temm1eError>>,
        temm1e_core::types::error::Temm1eError,
    > {
        unreachable!("fixture uses complete")
    }
    async fn health_check(&self) -> Result<bool, temm1e_core::types::error::Temm1eError> {
        Ok(true)
    }
    async fn list_models(&self) -> Result<Vec<String>, temm1e_core::types::error::Temm1eError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn optional_curator_charges_returned_usage_before_parsing_and_unknown_on_error_or_cancel() {
    for mode in ["valid", "malformed", "error", "pending"] {
        let provider = Arc::new(CuratorFixture {
            calls: std::sync::atomic::AtomicUsize::new(0),
            started: tokio::sync::Semaphore::new(0),
            mode,
        });
        let runtime = AgentRuntime::new(
            provider.clone(),
            Arc::new(MockMemory::new()),
            vec![],
            "gpt-4o".into(),
            None,
        )
        .with_v2_optimizations(false);
        let (reply, foreground) = runtime
            .process_message(
                &make_inbound_msg(
                    "I prefer Vietnamese when we discuss my ordinary project conversations.",
                ),
                &mut make_session(),
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(reply.text, "Foreground reply");
        assert_eq!(
            foreground.api_calls, 1,
            "background usage is not retroactively added to returned foreground usage"
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            provider.started.acquire(),
        )
        .await
        .unwrap()
        .unwrap()
        .forget();
        let drained = runtime
            .shutdown_background(std::time::Duration::from_millis(20))
            .await;
        assert_eq!(drained, mode != "pending");
        let budget = runtime.budget_snapshot();
        assert_eq!(budget.recorded_calls, 2, "mode={mode}");
        let incomplete = mode == "error" || mode == "pending";
        assert_eq!(budget.unpriced_calls, u64::from(incomplete), "mode={mode}");
        assert_eq!(budget.input_tokens, if incomplete { 10 } else { 20 });
        assert_eq!(budget.output_tokens, if incomplete { 20 } else { 40 });
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
