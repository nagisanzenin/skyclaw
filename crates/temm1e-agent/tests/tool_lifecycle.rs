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
