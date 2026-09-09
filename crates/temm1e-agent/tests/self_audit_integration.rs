//! Integration tests for the Self-Audit Pass (GH-62).
//!
//! Verifies the full agent loop with a queued multi-response mock that
//! simulates each scenario the audit gate is meant to handle:
//!   - Stalled-promise text → audit triggers tool call → loop continues
//!   - "[DONE]" audit response → exit with original pre-audit text
//!   - Malformed audit response → fail open (identical to baseline)
//!   - Disabled flag → no audit even on text-only exits
//!   - No tools available → skip audit (no point asking the model to
//!     emit a tool it can't use)
//!   - Already responded via send_message → skip audit
//!   - Hard cap = 1 audit per turn (cannot infinite-loop)

use std::sync::Arc;

use temm1e_agent::AgentRuntime;
use temm1e_core::{Memory, Tool};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, MockTool, QueuedMockProvider};

fn build_runtime(
    provider: Arc<QueuedMockProvider>,
    tools: Vec<Arc<dyn Tool>>,
    audit: bool,
) -> AgentRuntime {
    let memory = Arc::new(MockMemory::new());
    AgentRuntime::new(
        provider,
        memory,
        tools,
        "queued-mock-model".to_string(),
        Some("You are a test agent.".to_string()),
    )
    .with_v2_optimizations(false)
    .with_self_audit_enabled(audit)
}

#[tokio::test]
async fn audit_catches_stalled_promise() {
    // Round 1: text-only "Let me check" (the stalled promise).
    // Round 2: tool call (audit successfully prompted commitment).
    // Round 3: final text after tool ran.
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("Let me check the file"),
        QueuedMockProvider::tool_use_response("tu1", "mock_check", serde_json::json!({})),
        QueuedMockProvider::text_response("Found 3 endpoints in the file."),
    ]));
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool::new("mock_check"))];
    let runtime = build_runtime(provider.clone(), tools, true);

    let msg = make_inbound_msg("check the file");
    let mut session = make_session();

    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        provider.calls().await,
        3,
        "expected 3 provider calls (text + audit-tool + final), got {}",
        provider.calls().await
    );
    // Final user-facing text should be the round-3 answer, not the
    // round-1 stalled promise.
    assert!(
        reply.text.contains("Found 3 endpoints"),
        "user-facing reply should be the post-tool answer, got: {}",
        reply.text
    );
}

#[tokio::test]
async fn audit_done_token_serves_pre_audit_text() {
    // Round 1: genuine final answer.
    // Round 2: audit responds with "[DONE]" — model confirms it's done.
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("Hello, your name is Alice."),
        QueuedMockProvider::text_response("[DONE]"),
    ]));
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool::new("mock_lookup"))];
    let runtime = build_runtime(provider.clone(), tools, true);

    let msg = make_inbound_msg("what is my name?");
    let mut session = make_session();

    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(provider.calls().await, 2, "expected text + audit calls");
    assert_eq!(
        reply.text, "Hello, your name is Alice.",
        "should serve the pre-audit text, not [DONE]"
    );
    assert!(
        !reply.text.contains("[DONE]"),
        "[DONE] token must never reach the user"
    );
}

#[tokio::test]
async fn audit_preserves_correction_instead_of_restoring_original_claim() {
    // Round 1: text answer.
    // Round 2: malformed audit response (no [DONE], no tool call).
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("All tests passed."),
        QueuedMockProvider::text_response(
            "Correction: I did not run the tests. Their status is unverified.",
        ),
    ]));
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool::new("mock_tool"))];
    let runtime = build_runtime(provider.clone(), tools, true);

    let msg = make_inbound_msg("question");
    let mut session = make_session();

    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(provider.calls().await, 2);
    assert_eq!(
        reply.text, "Correction: I did not run the tests. Their status is unverified.",
        "the audit correction must reach the user"
    );
}

#[tokio::test]
async fn audit_disabled_no_extra_round() {
    // With audit OFF, identical to v5.5.5 baseline: 1 provider call,
    // text-only reply.
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("Let me check that"),
    ]));
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool::new("mock_tool"))];
    let runtime = build_runtime(provider.clone(), tools, false);

    let msg = make_inbound_msg("do thing");
    let mut session = make_session();

    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        provider.calls().await,
        1,
        "audit disabled — exactly one provider call"
    );
    assert_eq!(reply.text, "Let me check that");
}

#[tokio::test]
async fn audit_skipped_when_no_tools_available() {
    // No tools registered → nothing for the audit to ask the model
    // to call. Skip even when flag is on.
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("Pure conversational reply."),
    ]));
    let tools: Vec<Arc<dyn Tool>> = vec![];
    let runtime = build_runtime(provider.clone(), tools, true);

    let msg = make_inbound_msg("hi");
    let mut session = make_session();

    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        provider.calls().await,
        1,
        "no tools → audit skipped → exactly one provider call"
    );
    assert_eq!(reply.text, "Pure conversational reply.");
}

#[tokio::test]
async fn audit_hard_cap_one_per_turn() {
    // Round 1: text-only stalled promise.
    // Round 2: audit response is ALSO text-only (model didn't [DONE]
    //          and didn't tool-call). At this point the audit gate
    //          MUST NOT fire again — it would loop forever otherwise.
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("Let me think"),
        QueuedMockProvider::text_response("Still thinking"),
        // If the cap were broken, round 3+ would be requested and
        // panic with "QueuedMockProvider exhausted" (caught by the
        // cap, never reached).
    ]));
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool::new("mock_tool"))];
    let runtime = build_runtime(provider.clone(), tools, true);

    let msg = make_inbound_msg("do thing");
    let mut session = make_session();

    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        provider.calls().await,
        2,
        "audit must be capped at 1 — exactly 2 provider calls total"
    );
    assert_eq!(
        reply.text, "Still thinking",
        "the audit text is retained without another audit round"
    );
}

#[tokio::test]
async fn audit_records_telemetry() {
    // Verify that on a [DONE] audit, the model_discipline counters tick.
    use temm1e_memory::SqliteMemory;

    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response("All set."),
        QueuedMockProvider::text_response("[DONE]"),
    ]));
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(MockTool::new("mock_tool"))];
    let memory = Arc::new(SqliteMemory::new("sqlite::memory:").await.unwrap());
    let runtime = AgentRuntime::new(
        provider.clone(),
        memory.clone(),
        tools,
        "queued-mock-model".to_string(),
        Some("test".to_string()),
    )
    .with_v2_optimizations(false)
    .with_self_audit_enabled(true);

    let msg = make_inbound_msg("hello");
    let mut session = make_session();
    runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    // Telemetry recording is fire-and-forget via tokio::spawn — give it
    // a moment to land. Poll up to 1s.
    let mut discipline = None;
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(Some(d)) = memory
            .get_model_discipline("queued-mock", "queued-mock-model")
            .await
        {
            discipline = Some(d);
            break;
        }
    }
    let d = discipline.expect("audit telemetry should have been recorded");
    assert_eq!(d.text_only_exits, 1);
    assert_eq!(d.audit_done_responses, 1);
    assert_eq!(d.audit_tool_call_responses, 0);
    assert_eq!(d.audit_failed_responses, 0);
}
