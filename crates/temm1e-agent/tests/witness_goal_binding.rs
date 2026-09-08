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

#[cfg(unix)]
#[tokio::test]
async fn user_role_cannot_launch_a_process_through_automatic_witness_checks() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("unauthorized-verifier-marker");
    let command = format!(
        "printf verifier-ran > '{}'",
        marker.display().to_string().replace('\'', "'\\''")
    );
    let proposed = serde_json::json!({"goal":"model proposal", "postconditions":[{
        "kind":"command_exits", "cmd":"sh", "args":["-c",command], "expected_code":0,
        "cwd":null, "timeout_ms":2000
    }]})
    .to_string();
    let witness = Arc::new(Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    ));
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response(&proposed),
        QueuedMockProvider::text_response("unverified reply"),
    ]));
    let runtime = AgentRuntime::new(
        provider,
        Arc::new(MockMemory::new()),
        vec![],
        "fixture".into(),
        None,
    )
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_witness(witness.clone(), WitnessStrictness::Observe, false)
    .with_auto_planner_oath(true);
    let mut session = make_session();
    session.workspace_path = directory.path().into();
    session.role = temm1e_core::types::rbac::Role::User;
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
    assert!(
        runtime
            .shutdown_background(std::time::Duration::from_secs(1))
            .await
    );
    assert!(
        !marker.exists(),
        "User bypassed shell authority through Witness"
    );
    let verdicts: Vec<_> = witness
        .ledger()
        .read_session(&session.session_id)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry.payload {
            LedgerPayload::VerdictRendered(verdict) => Some(verdict),
            _ => None,
        })
        .collect();
    assert_eq!(verdicts.len(), 1);
    assert_eq!(
        verdicts[0].outcome,
        temm1e_witness::types::VerdictOutcome::Inconclusive
    );
    assert_eq!(verdicts[0].tier_usage.tier0_calls, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn witness_command_authority_covers_composites_without_changing_admin_or_file_checks() {
    use temm1e_core::types::rbac::Role;
    use temm1e_witness::types::{Predicate, VerdictOutcome};
    for kind in [
        "exit", "contains", "absent", "duration", "all", "any", "not",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("owned-command-marker");
        let script = format!(
            "printf observed; printf effect > '{}'",
            marker.display().to_string().replace('\'', "'\\''")
        );
        let command = Predicate::CommandExits {
            cmd: "sh".into(),
            args: vec!["-c".into(), script.clone()],
            expected_code: 0,
            cwd: None,
            timeout_ms: 2000,
        };
        let predicate = match kind {
            "contains" => Predicate::CommandOutputContains {
                cmd: "sh".into(),
                args: vec!["-c".into(), script.clone()],
                regex: "observed".into(),
                stream: temm1e_witness::types::OutputStream::Stdout,
                cwd: None,
                timeout_ms: 2000,
            },
            "absent" => Predicate::CommandOutputAbsent {
                cmd: "sh".into(),
                args: vec!["-c".into(), script.clone()],
                regex: "unexpected".into(),
                stream: temm1e_witness::types::OutputStream::Stdout,
                cwd: None,
                timeout_ms: 2000,
            },
            "duration" => Predicate::CommandDurationUnder {
                cmd: "sh".into(),
                args: vec!["-c".into(), script],
                max_ms: 2000,
                cwd: None,
            },
            "all" => Predicate::AllOf {
                predicates: vec![command.clone()],
            },
            "any" => Predicate::AnyOf {
                predicates: vec![command.clone()],
            },
            "not" => Predicate::NotOf {
                predicate: Box::new(command.clone()),
            },
            _ => command,
        };
        let witness = Witness::new(
            Ledger::open("sqlite::memory:").await.unwrap(),
            directory.path(),
        );
        // Trusted explicit host goal text; this test exercises evaluator authority,
        // while the separate full-runtime test exercises planner admission.
        let oath = Oath::draft(
            format!("sub-{kind}"),
            format!("goal-{kind}"),
            "session",
            "inspect fixture",
        )
        .with_postcondition(predicate)
        .with_postcondition(Predicate::DirectoryExists {
            path: directory.path().into(),
        });
        let (oath, _) = temm1e_witness::oath::seal_oath(witness.ledger(), oath)
            .await
            .unwrap();
        let user = witness.for_authority(directory.path(), Role::User);
        let verdict = user.verify_oath(&oath).await.unwrap();
        assert!(!marker.exists(), "{kind} executed for User");
        assert_eq!(
            verdict.per_predicate[0].outcome,
            VerdictOutcome::Inconclusive,
            "{kind}"
        );
        assert_eq!(verdict.per_predicate[1].outcome, VerdictOutcome::Pass);
        assert_eq!(verdict.tier_usage.tier0_calls, 1); // Only directory check actually ran.
        let rebound = user
            .for_authority(directory.path(), Role::Admin)
            .verify_oath(&oath)
            .await
            .unwrap();
        assert_eq!(
            rebound.per_predicate[0].outcome,
            VerdictOutcome::Inconclusive
        );
        assert!(!marker.exists(), "rebinding expanded existing authority");
        let admin = witness
            .for_authority(directory.path(), Role::Admin)
            .verify_oath(&oath)
            .await
            .unwrap();
        assert!(marker.exists(), "{kind} stopped authorized Admin execution");
        assert_eq!(
            admin.per_predicate[0].outcome,
            if kind == "not" {
                VerdictOutcome::Fail
            } else {
                VerdictOutcome::Pass
            }
        );
    }
}

#[tokio::test]
async fn restricted_witness_denies_command_checks_before_platform_process_launch() {
    use temm1e_core::types::rbac::Role;
    use temm1e_witness::types::{Predicate, VerdictOutcome};
    let directory = tempfile::tempdir().unwrap();
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    );
    let command = Predicate::CommandExits {
        cmd: directory
            .path()
            .join("not-created-program")
            .to_string_lossy()
            .into_owned(),
        args: vec![],
        expected_code: 0,
        cwd: None,
        timeout_ms: 1000,
    };
    for (index, predicate) in [
        command.clone(),
        Predicate::AllOf {
            predicates: vec![command.clone()],
        },
        Predicate::AnyOf {
            predicates: vec![command.clone()],
        },
        Predicate::NotOf {
            predicate: Box::new(command),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let oath = Oath::draft(
            format!("sub-{index}"),
            format!("goal-{index}"),
            "session",
            "inspect fixture",
        )
        .with_postcondition(predicate);
        let (oath, _) = temm1e_witness::oath::seal_oath(witness.ledger(), oath)
            .await
            .unwrap();
        let verdict = witness
            .for_authority(directory.path(), Role::User)
            .verify_oath(&oath)
            .await
            .unwrap();
        assert_eq!(verdict.outcome, VerdictOutcome::Inconclusive);
        assert_eq!(verdict.tier_usage.tier0_calls, 0);
        assert!(verdict.per_predicate[0].detail.contains("shell permission"));
    }
}

#[tokio::test]
async fn automatic_oath_is_persisted_with_the_active_goal_before_return() {
    use temm1e_agent::{conversation::ConversationScope, execution_journal::ExecutionJournal};
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture.txt"), "token token").unwrap();
    let path = directory.path().join("executions.db");
    let journal = Arc::new(ExecutionJournal::open(&path).await.unwrap());
    let scope = ConversationScope::new(directory.path(), "test", "test-chat", "test-user").unwrap();
    let turn = journal.acquire_conversation(&scope).await.unwrap();
    let witness = Arc::new(Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path().to_path_buf(),
    ));
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response(&draft()),
        QueuedMockProvider::text_response("DONE despite incomplete original request"),
    ]));
    let runtime = AgentRuntime::new(
        provider,
        Arc::new(MockMemory::new()),
        vec![],
        "fixture".into(),
        Some("local fixture".into()),
    )
    .with_execution_journal(journal.clone())
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false)
    .with_witness(witness, WitnessStrictness::Observe, false)
    .with_auto_planner_oath(true);
    let mut session = make_session();
    session.workspace_path = directory.path().into();
    session.session_id = turn.epoch().into();
    session.channel = "test".into();
    session.chat_id = "test-chat".into();
    session.user_id = "test-user".into();
    let original = "In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String. Preserve COMPLETE_REQUIREMENT.";
    runtime
        .process_message(
            &make_inbound_msg(original),
            &mut session,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let pool =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
    let (id, state, revision): (String, String, i64) =
        sqlx::query_as("SELECT id,state,revision FROM goal_records")
            .fetch_one(&pool)
            .await
            .unwrap();
    let document: String = sqlx::query_scalar("SELECT document FROM goal_criteria WHERE goal_id=?")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let saved: serde_json::Value = serde_json::from_str(&document).unwrap();
    assert_eq!(saved["origin"], "model_proposed");
    assert_eq!(saved["coverage"], "unverified");
    assert_eq!(saved["oath"]["goal"], original);
    assert_eq!(saved["oath"]["root_goal_id"], id);
    assert_eq!(state, "awaiting_evidence");
    assert_eq!(revision, 2);
}
