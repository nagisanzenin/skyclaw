//! Actual planner/runtime/Witness binding against an isolated SQLite ledger.
use std::{collections::HashSet, sync::Arc};
use temm1e_agent::{budget::BudgetTracker, AgentRuntime};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, QueuedMockProvider};
use temm1e_witness::{
    config::WitnessStrictness,
    ledger::Ledger,
    planner::{seal_oath_via_planner, PlannerOathRequest},
    types::{LedgerPayload, Oath},
    Witness,
};

fn draft() -> String {
    serde_json::json!({"goal":"a much weaker model-written goal", "postconditions":[
        {"kind":"file_exists","path":"fixture.txt"},
        {"kind":"grep_count_at_least","pattern":"token","path_glob":"*.txt","n":2},
        {"kind":"grep_absent","pattern":"TODO","path_glob":"*.txt"}
    ]})
    .to_string()
}

#[tokio::test]
async fn planner_cannot_replace_the_original_user_objective() {
    let directory = tempfile::tempdir().unwrap();
    let witness = Arc::new(Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path().to_path_buf(),
    ));
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response(&draft()),
    ]));
    let original = "In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String. Preserve REQUIREMENT_SENTINEL.";
    let (sealed, _) = seal_oath_via_planner(PlannerOathRequest {
        witness: &witness,
        provider,
        model: "fixture".into(),
        user_request: original,
        workspace_root: directory.path(),
        session_id: "same-session".into(),
        root_goal_id: "execution-one".into(),
        subtask_id: "criterion-set-one".into(),
    })
    .await
    .unwrap();
    assert_eq!(sealed.goal, original);
}

async fn two_turns() -> (Vec<Oath>, u64, usize) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture.txt"), "token token").unwrap();
    let witness = Arc::new(Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path().to_path_buf(),
    ));
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response(&draft()),
        QueuedMockProvider::text_response("first reply"),
        QueuedMockProvider::text_response(&draft()),
        QueuedMockProvider::text_response("second reply"),
    ]));
    let budget = Arc::new(BudgetTracker::new(0.0));
    let runtime = AgentRuntime::new(
        provider.clone(),
        Arc::new(MockMemory::new()),
        vec![],
        "fixture".into(),
        Some("local fixture".into()),
    )
    .with_budget(budget.clone())
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_witness(witness.clone(), WitnessStrictness::Observe, false)
    .with_auto_planner_oath(true);
    let mut session = make_session();
    session.workspace_path = directory.path().into();
    for index in 0..2 {
        let msg = make_inbound_msg(&format!("In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String. Requirement {index}."));
        runtime
            .process_message(&msg, &mut session, None, None, None, None, None)
            .await
            .unwrap();
    }
    assert!(
        runtime
            .shutdown_background(std::time::Duration::from_secs(1))
            .await
    );
    let oaths = witness
        .ledger()
        .read_session(&session.session_id)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry.payload {
            LedgerPayload::OathSealed(oath) => Some(oath),
            _ => None,
        })
        .collect();
    (
        oaths,
        budget.snapshot().recorded_calls,
        provider.calls().await,
    )
}

#[tokio::test]
async fn separate_turns_in_one_session_never_share_witness_goal_or_subtask_ids() {
    let (oaths, _, calls) = two_turns().await;
    assert_eq!(calls, 4);
    assert_eq!(oaths.len(), 2);
    assert_eq!(
        oaths
            .iter()
            .map(|o| &o.root_goal_id)
            .collect::<HashSet<_>>()
            .len(),
        2
    );
    assert_eq!(
        oaths
            .iter()
            .map(|o| &o.subtask_id)
            .collect::<HashSet<_>>()
            .len(),
        2
    );
}

#[tokio::test]
async fn planner_calls_are_charged_to_the_owning_runtime() {
    let (_, recorded, calls) = two_turns().await;
    assert_eq!(calls, 4);
    assert_eq!(recorded, calls as u64, "planner usage was discarded");
}

struct PlannerFailureFixture {
    mode: &'static str,
    started: tokio::sync::Notify,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl temm1e_core::Provider for PlannerFailureFixture {
    fn name(&self) -> &str {
        "openai"
    }
    async fn complete(
        &self,
        request: temm1e_core::types::message::CompletionRequest,
    ) -> Result<
        temm1e_core::types::message::CompletionResponse,
        temm1e_core::types::error::Temm1eError,
    > {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let planner = request
            .system
            .as_deref()
            .is_some_and(|s| s.starts_with("You are the Oath Planner"));
        if planner {
            self.started.notify_one();
            if self.mode == "error" {
                return Err(temm1e_core::types::error::Temm1eError::Provider(
                    "synthetic planner error".into(),
                ));
            }
            if self.mode == "pending" {
                std::future::pending::<()>().await;
            }
        }
        let mut response = QueuedMockProvider::text_response(if planner {
            "malformed planner JSON"
        } else {
            "foreground reply"
        });
        response.usage.totals_reported = Some(true);
        Ok(response)
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
        unreachable!()
    }
    async fn health_check(&self) -> Result<bool, temm1e_core::types::error::Temm1eError> {
        Ok(true)
    }
    async fn list_models(&self) -> Result<Vec<String>, temm1e_core::types::error::Temm1eError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn malformed_failed_and_cancelled_planners_preserve_knownness_once() {
    for mode in ["malformed", "error", "pending"] {
        let directory = tempfile::tempdir().unwrap();
        let witness = Arc::new(Witness::new(
            Ledger::open("sqlite::memory:").await.unwrap(),
            directory.path(),
        ));
        let provider = Arc::new(PlannerFailureFixture {
            mode,
            started: tokio::sync::Notify::new(),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let runtime = AgentRuntime::new(
            provider.clone(),
            Arc::new(MockMemory::new()),
            vec![],
            "gpt-4o".into(),
            None,
        )
        .with_v2_optimizations(false)
        .with_self_audit_enabled(false)
        .with_witness(witness, WitnessStrictness::Observe, false)
        .with_auto_planner_oath(true);
        let mut session = make_session();
        session.workspace_path = directory.path().into();
        let msg = make_inbound_msg(
            "In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String.",
        );
        if mode == "pending" {
            let cancel = tokio_util::sync::CancellationToken::new();
            let interrupt = async {
                provider.started.notified().await;
                cancel.cancel();
            };
            let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::join!(
                    runtime.process_message(
                        &msg,
                        &mut session,
                        None,
                        None,
                        None,
                        None,
                        Some(cancel.clone())
                    ),
                    interrupt
                )
            })
            .await
            .unwrap();
            assert!(result.is_err());
        } else {
            runtime
                .process_message(&msg, &mut session, None, None, None, None, None)
                .await
                .unwrap();
        }
        assert!(
            runtime
                .shutdown_background(std::time::Duration::from_secs(1))
                .await
        );
        let snapshot = runtime.budget_snapshot();
        let pending = mode == "pending";
        assert_eq!(
            snapshot.recorded_calls,
            if pending { 1 } else { 2 },
            "{mode}"
        );
        assert_eq!(
            snapshot.unpriced_calls,
            u64::from(mode != "malformed"),
            "{mode}"
        );
        assert_eq!(
            snapshot.input_tokens,
            if pending {
                0
            } else if mode == "error" {
                10
            } else {
                20
            }
        );
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst) as u64,
            snapshot.recorded_calls
        );
    }
}
