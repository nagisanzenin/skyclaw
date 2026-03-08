//! Integration tests for the agent runtime — tests the full message processing
//! loop with mock provider and mock memory working together.

use std::sync::Arc;

use skyclaw_agent::AgentRuntime;
use skyclaw_core::types::message::*;
use skyclaw_core::Tool;
use skyclaw_test_utils::{make_inbound_msg, make_session, MockMemory, MockProvider, MockTool};

fn make_runtime_with_text(text: &str) -> AgentRuntime {
    let provider = Arc::new(MockProvider::with_text(text));
    let memory = Arc::new(MockMemory::new());
    let tools: Vec<Arc<dyn Tool>> = vec![];

    AgentRuntime::new(
        provider,
        memory,
        tools,
        "test-model".to_string(),
        Some("You are a test agent.".to_string()),
    )
}

#[tokio::test]
async fn simple_text_response() {
    let runtime = make_runtime_with_text("Hello from the AI!");
    let msg = make_inbound_msg("Hi there");
    let mut session = make_session();

    let reply = runtime
        .process_message(&msg, &mut session, None, None)
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
        .process_message(&msg, &mut session, None, None)
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

    let reply = runtime
        .process_message(&msg, &mut session, None, None)
        .await
        .unwrap();
    // Empty message with no attachments returns a friendly error
    assert!(reply.text.contains("empty message"));
}

#[tokio::test]
async fn provider_called_exactly_once_for_simple_text() {
    let provider = Arc::new(MockProvider::with_text("response"));
    let memory = Arc::new(MockMemory::new());
    let runtime = AgentRuntime::new(provider.clone(), memory, vec![], "model".to_string(), None);

    let msg = make_inbound_msg("hello");
    let mut session = make_session();
    runtime
        .process_message(&msg, &mut session, None, None)
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
        skyclaw_test_utils::make_test_entry_with_session(
            "mem1",
            "Important context about Rust",
            "test:test-chat:test-user",
        ),
    ]));

    let provider = Arc::new(MockProvider::with_text("I remember about Rust!"));
    let runtime = AgentRuntime::new(provider.clone(), memory, vec![], "model".to_string(), None);

    let msg = make_inbound_msg("Tell me about Rust");
    let mut session = make_session();
    let reply = runtime
        .process_message(&msg, &mut session, None, None)
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
        let reply = runtime
            .process_message(&msg, &mut session, None, None)
            .await
            .unwrap();
        assert_eq!(reply.text, "Reply");
    }

    // History should have 3 user + 3 assistant = 6 messages
    assert_eq!(session.history.len(), 6);
}
