use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use temm1e_agent::AgentRuntime;
use temm1e_core::types::{
    error::Temm1eError,
    message::{ContentPart, MessageContent},
};
use temm1e_core::{Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, QueuedMockProvider};
use tokio_util::sync::CancellationToken;

struct SlowTool {
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<AtomicBool>,
    effect: Arc<AtomicBool>,
}
struct DropProbe(Arc<AtomicBool>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[async_trait]
impl Tool for SlowTool {
    fn name(&self) -> &str {
        "slow_fixture"
    }
    fn description(&self) -> &str {
        "Cancellation fixture"
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
        let _probe = DropProbe(self.dropped.clone());
        self.started.notify_one();
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        self.effect.store(true, Ordering::SeqCst);
        Ok(ToolOutput {
            content: "finished".into(),
            is_error: false,
        })
    }
}

#[tokio::test]
async fn token_flag_and_deadline_interrupt_in_flight_tools_and_preserve_uncertainty() {
    for mechanism in ["token", "legacy", "deadline"] {
        let started = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let effect = Arc::new(AtomicBool::new(false));
        let tool = Arc::new(SlowTool {
            started: started.clone(),
            dropped: dropped.clone(),
            effect: effect.clone(),
        });
        let provider = Arc::new(QueuedMockProvider::with_responses(vec![
            QueuedMockProvider::tool_use_response("call", "slow_fixture", serde_json::json!({})),
        ]));
        let directory = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            temm1e_agent::execution_journal::ExecutionJournal::open(
                &directory.path().join("executions.db"),
            )
            .await
            .unwrap(),
        );
        let runtime = AgentRuntime::with_limits(
            provider,
            Arc::new(MockMemory::new()),
            vec![tool],
            "fixture-model".into(),
            Some("Test".into()),
            10,
            8000,
            6,
            if mechanism == "deadline" { 1 } else { 0 },
            0.0,
        )
        .with_execution_journal(journal.clone())
        .with_v2_optimizations(false)
        .with_self_audit_enabled(false);
        let token = CancellationToken::new();
        let flag = Arc::new(AtomicBool::new(false));
        let stop_token = token.clone();
        let stop_flag = flag.clone();
        let stop = tokio::spawn(async move {
            started.notified().await;
            if mechanism == "legacy" {
                stop_flag.store(true, Ordering::Relaxed);
            } else if mechanism == "token" {
                stop_token.cancel();
            }
        });
        let mut session = make_session();
        session.workspace_path = directory.path().to_owned();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            runtime.process_message(
                &make_inbound_msg("Run slow fixture"),
                &mut session,
                Some(flag),
                None,
                None,
                None,
                Some(token),
            ),
        )
        .await
        .expect("cancellation must not wait for tool completion");
        let expected = if mechanism == "deadline" {
            "Task deadline reached"
        } else {
            "Task stopped"
        };
        assert!(result.unwrap_err().to_string().contains(expected));
        stop.await.unwrap();
        let unfinished = journal.unfinished(&session).await.unwrap();
        assert_eq!(unfinished.len(), 1);
        assert_eq!(unfinished[0].state, "interrupted");
        assert!(unfinished[0].checkpoint.contains("Outcome unknown"));
        assert!(dropped.load(Ordering::SeqCst));
        assert!(!effect.load(Ordering::SeqCst));
        assert!(session
            .history
            .iter()
            .any(|message| match &message.content {
                MessageContent::Parts(parts) => parts.iter().any(|p| matches!(p,
                ContentPart::ToolResult { tool_use_id, content, is_error: true }
                    if tool_use_id == "call" && content.contains("Outcome unknown"))),
                _ => false,
            }));
    }
}
