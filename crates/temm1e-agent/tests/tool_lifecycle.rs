use std::sync::{Arc, Mutex};
use temm1e_agent::{agent_task_status::AgentTaskPhase, AgentRuntime};
use temm1e_core::Tool;
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, MockTool, QueuedMockProvider};

#[tokio::test]
async fn fast_repeated_tools_emit_distinct_ordered_events_without_watch_receiver() {
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::tool_use_response(
            "reused-model-id",
            "mock_check",
            serde_json::json!({"step": 1}),
        ),
        QueuedMockProvider::tool_use_response(
            "reused-model-id",
            "mock_check",
            serde_json::json!({"step": 2}),
        ),
        QueuedMockProvider::text_response("Checks finished."),
    ]));
    let events = Arc::new(Mutex::new(Vec::new()));
    let observer_events = events.clone();
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool::new("mock_check"))];
    let runtime = AgentRuntime::new(
        provider,
        Arc::new(MockMemory::new()),
        tools,
        "queued-mock-model".into(),
        Some("Test agent".into()),
    )
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_tool_observer(Arc::new(move |event| {
        observer_events.lock().unwrap().push(event)
    }));
    runtime
        .process_message(
            &make_inbound_msg("Run the checks"),
            &mut make_session(),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 4);
    for pair in events.chunks_exact(2) {
        assert_eq!(pair[0].execution_id, pair[1].execution_id);
        assert!(matches!(
            pair[0].phase,
            AgentTaskPhase::ExecutingTool { .. }
        ));
        assert!(matches!(
            pair[1].phase,
            AgentTaskPhase::ToolCompleted { ok: true, .. }
        ));
    }
    assert_ne!(events[0].execution_id, events[2].execution_id);
}

#[tokio::test]
async fn runtime_filter_is_enforced_at_dispatch_even_if_model_invents_hidden_tool() {
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use temm1e_core::{
        types::error::Temm1eError, ToolContext, ToolDeclarations, ToolInput, ToolOutput,
    };
    struct Counted(Arc<AtomicUsize>);
    #[async_trait]
    impl Tool for Counted {
        fn name(&self) -> &str {
            "hidden_effect"
        }
        fn description(&self) -> &str {
            "Must not be dispatched when filtered"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type":"object"})
        }
        fn declarations(&self) -> ToolDeclarations {
            ToolDeclarations {
                file_access: vec![],
                network_access: vec![],
                shell_access: false,
            }
        }
        async fn execute(&self, _: ToolInput, _: &ToolContext) -> Result<ToolOutput, Temm1eError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput {
                content: "effect happened".into(),
                is_error: false,
            })
        }
    }
    let count = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::tool_use_response("x", "hidden_effect", serde_json::json!({})),
        QueuedMockProvider::text_response("The hidden tool was unavailable."),
    ]));
    let runtime = AgentRuntime::new(
        provider,
        Arc::new(MockMemory::new()),
        vec![Arc::new(Counted(count.clone()))],
        "fixture".into(),
        Some("Test".into()),
    )
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_tool_filter(Arc::new(|_| false));
    let mut session = make_session();
    runtime
        .process_message(
            &make_inbound_msg("Run the action"),
            &mut session,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
    let history = serde_json::to_string(&session.history).unwrap();
    assert!(!history.contains("effect happened"));
}
