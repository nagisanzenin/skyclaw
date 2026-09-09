//! End-to-end planner -> file evidence -> evaluator -> durable scoped assessment.
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use temm1e_agent::{
    conversation::ConversationScope, execution_journal::ExecutionJournal, AgentRuntime,
};
use temm1e_core::types::goal::{AssessmentOutcome, GoalState};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, QueuedMockProvider};
use temm1e_witness::{
    config::WitnessStrictness,
    ledger::Ledger,
    witness::{LlmVerifierResponse, Tier1Verifier},
    Witness, WitnessError,
};

#[derive(Default)]
struct FileReviewer(Mutex<Vec<String>>);
#[async_trait]
impl Tier1Verifier for FileReviewer {
    async fn verify(
        &self,
        _: &str,
        _: &str,
        evidence: &str,
    ) -> Result<LlmVerifierResponse, WitnessError> {
        self.0.lock().unwrap().push(evidence.into());
        assert!(evidence.contains("SOURCE_VERSION_ONE_53"));
        Ok(LlmVerifierResponse {
            verdict: "pass".into(),
            reason: "fixture reviewed only supplied bytes".into(),
        })
    }
}

#[tokio::test]
async fn actual_runtime_retains_inspected_bytes_after_source_changes_and_rejects_tampering() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("executions.db");
    std::fs::write(
        directory.path().join("artifact.txt"),
        "token token SOURCE_VERSION_ONE_53",
    )
    .unwrap();
    let journal = Arc::new(ExecutionJournal::open(&path).await.unwrap());
    let scope = ConversationScope::new(directory.path(), "test", "room", "alice").unwrap();
    let turn = journal.acquire_conversation(&scope).await.unwrap();
    let reviewer = Arc::new(FileReviewer::default());
    let witness = Arc::new(
        Witness::new(
            Ledger::open("sqlite::memory:").await.unwrap(),
            directory.path(),
        )
        .with_tier1(reviewer.clone()),
    );
    let draft = serde_json::json!({"goal":"weaker model draft", "postconditions":[
        {"kind":"file_exists","path":"artifact.txt"},
        {"kind":"grep_count_at_least","pattern":"token","path_glob":"*.txt","n":2},
        {"kind":"grep_absent","pattern":"TODO","path_glob":"*.txt"},
        {"kind":"aspect_verifier","rubric":"Review the supplied artifact","evidence_refs":["artifact"],"advisory":false}],
        "evidence_required":[{"id":"artifact","kind":{"kind":"file","path":"artifact.txt"},"description":"actual bytes"}]}).to_string();
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::text_response(&draft),
        QueuedMockProvider::text_response("DONE claim is not proof"),
    ]));
    let runtime = AgentRuntime::new(
        provider,
        Arc::new(MockMemory::new()),
        vec![],
        "fixture".into(),
        Some("fixture".into()),
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
    session.chat_id = "room".into();
    session.user_id = "alice".into();
    let original = "In the workspace, write `demo.rs` with pub fn greet(name: &str) -> String. Preserve the complete original requirement.";
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
    assert_eq!(reviewer.0.lock().unwrap().len(), 1);
    let goals = journal.goal_status(&scope).await.unwrap();
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].state, GoalState::AwaitingEvidence);
    assert_eq!(goals[0].objective, original);
    let id = &goals[0].id;
    std::fs::write(
        directory.path().join("artifact.txt"),
        "SOURCE_VERSION_TWO_53",
    )
    .unwrap();
    let reopened = ExecutionJournal::open(&path).await.unwrap();
    let saved = reopened.goal_assessment(&scope, id).await.unwrap().unwrap();
    assert_eq!(saved.declared_outcome, AssessmentOutcome::Passed);
    assert_eq!(saved.model_evidence.len(), 1);
    assert_eq!(saved.model_evidence[0].predicate_index, 3);
    let snapshot = &saved.model_evidence[0].snapshots[0];
    assert!(snapshot.content.contains("SOURCE_VERSION_ONE_53"));
    assert!(!snapshot.content.contains("SOURCE_VERSION_TWO_53"));
    assert_eq!(
        saved.observations[3].observation.evaluator_version,
        "temm1e-witness/model-file-evidence-v1"
    );
    let foreign = ConversationScope::new(directory.path(), "test", "room", "bob").unwrap();
    assert!(reopened
        .goal_assessment(&foreign, id)
        .await
        .unwrap()
        .is_none());
    // Even updating the outer document hash cannot hide a changed inner blob.
    let pool =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
    let mut document = serde_json::to_value(saved).unwrap();
    document["model_evidence"][0]["snapshots"][0]["content"] = "FORGED".into();
    let document = serde_json::to_string(&document).unwrap();
    let hash = hex::encode(Sha256::digest(document.as_bytes()));
    sqlx::query("UPDATE goal_assessments SET document=?,hash=? WHERE goal_id=?")
        .bind(document)
        .bind(hash)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(reopened.goal_assessment(&scope, id).await.is_err());
}
