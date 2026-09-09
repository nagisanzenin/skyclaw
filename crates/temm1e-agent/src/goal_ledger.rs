//! Active execution goal records. No background pursuit or success-from-prose.
use crate::{conversation::ConversationScope, execution_journal::ExecutionJournal};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, Transaction};
use temm1e_core::types::{
    error::Temm1eError,
    goal::{GoalState, ToolResultEvidence, GOAL_SCHEMA_VERSION},
};

fn error(e: impl std::fmt::Display) -> Temm1eError {
    Temm1eError::Internal(format!("Goal ledger: {e}"))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoalStatus {
    pub id: String,
    pub schema_version: u32,
    pub revision: i64,
    pub state: GoalState,
    pub objective: String,
    pub reason: String,
    pub evidence_count: i64,
    pub model_criteria_saved: bool,
    pub unresolved_operations: i64,
}

impl ExecutionJournal {
    pub(crate) async fn init_goals(pool: &sqlx::SqlitePool) -> Result<(), Temm1eError> {
        let mut tx = pool.begin().await.map_err(error)?;
        for statement in "CREATE TABLE IF NOT EXISTS goal_schema (singleton INTEGER PRIMARY KEY CHECK(singleton=1), version INTEGER NOT NULL);
            INSERT OR IGNORE INTO goal_schema(singleton,version) VALUES(1,1);".split(';').filter(|s| !s.trim().is_empty()) {
 sqlx::query(statement).execute(&mut *tx).await.map_err(error)?;
}
        let version: i64 = sqlx::query_scalar("SELECT version FROM goal_schema WHERE singleton=1")
            .fetch_one(&mut *tx)
            .await
            .map_err(error)?;
        if version != i64::from(GOAL_SCHEMA_VERSION) {
            return Err(error(
                "unsupported schema version; no downgrade was attempted",
            ));
        }
        for statement in "CREATE TABLE IF NOT EXISTS goal_records (
            id TEXT PRIMARY KEY REFERENCES executions(id), schema_version INTEGER NOT NULL,
            scope TEXT NOT NULL, conversation_scope TEXT, principal TEXT NOT NULL,
            objective TEXT NOT NULL, state TEXT NOT NULL, revision INTEGER NOT NULL,
            reason TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS goals_conversation ON goal_records(conversation_scope,updated_at);
            CREATE TABLE IF NOT EXISTS goal_events (
            goal_id TEXT NOT NULL REFERENCES goal_records(id), sequence INTEGER NOT NULL,
            revision INTEGER NOT NULL, kind TEXT NOT NULL, document TEXT NOT NULL, created_at TEXT NOT NULL,
            PRIMARY KEY(goal_id,sequence));
            CREATE TABLE IF NOT EXISTS goal_evidence (
            goal_id TEXT NOT NULL REFERENCES goal_records(id), hash TEXT NOT NULL,
            document TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY(goal_id,hash));
            CREATE TABLE IF NOT EXISTS goal_criteria (
            goal_id TEXT PRIMARY KEY REFERENCES goal_records(id), hash TEXT NOT NULL,
            document TEXT NOT NULL, created_at TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS goal_assessments (
            goal_id TEXT PRIMARY KEY REFERENCES goal_records(id), criteria_hash TEXT NOT NULL,
            hash TEXT NOT NULL, document TEXT NOT NULL, created_at TEXT NOT NULL);".split(';').filter(|s| !s.trim().is_empty()) {
 sqlx::query(statement).execute(&mut *tx).await.map_err(error)?;
}
        tx.commit().await.map_err(error)?;
        Ok(())
    }

    pub(crate) async fn goal_event(
        tx: &mut Transaction<'_, Sqlite>,
        id: &str,
        kind: &str,
        document: serde_json::Value,
    ) -> Result<(), Temm1eError> {
        let document = serde_json::to_string(&document).map_err(error)?;
        if document.len() > 8192 {
            return Err(error("event metadata exceeds 8 KiB"));
        }
        // No row for legacy executions: preserve them rather than invent goals.
        sqlx::query("INSERT INTO goal_events(goal_id,sequence,revision,kind,document,created_at)
            SELECT id,COALESCE((SELECT MAX(sequence) FROM goal_events WHERE goal_id=?),-1)+1,revision,?,?,?
            FROM goal_records WHERE id=?")
            .bind(id).bind(kind).bind(document).bind(chrono::Utc::now().to_rfc3339()).bind(id)
            .execute(&mut **tx).await.map_err(error)?;
        Ok(())
    }

    pub(crate) async fn goal_result_evidence(
        tx: &mut Transaction<'_, Sqlite>,
        id: &str,
        evidence: &ToolResultEvidence,
    ) -> Result<(), Temm1eError> {
        let document = serde_json::to_string(evidence).map_err(error)?;
        // JSON escaping can expand a bounded UTF-8 output by up to six times.
        if document.len() > 512 * 1024 {
            return Err(error("tool evidence exceeds 512 KiB serialized bound"));
        }
        let hash = hex::encode(Sha256::digest(document.as_bytes()));
        sqlx::query(
            "INSERT INTO goal_evidence(goal_id,hash,document,created_at)
            SELECT id,?,?,? FROM goal_records WHERE id=?",
        )
        .bind(&hash)
        .bind(document)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(error)?;
        Self::goal_event(
            tx,
            id,
            "tool_result_recorded",
            serde_json::json!({"operation_id":evidence.operation_id, "evidence_hash":hash}),
        )
        .await
    }

    /// Read-only bounded inspection by the authenticated conversation domain.
    /// A saved Running state is historical, not a live lease or replay permission.
    pub async fn goal_status(
        &self,
        scope: &ConversationScope,
    ) -> Result<Vec<GoalStatus>, Temm1eError> {
        type Row = (String, i64, i64, String, String, String, i64, i64, bool);
        let rows: Vec<Row> = sqlx::query_as("SELECT id,schema_version,revision,state,substr(objective,1,512),reason,
            (SELECT COUNT(*) FROM goal_evidence e WHERE e.goal_id=g.id),
            (SELECT COUNT(*) FROM execution_operations o WHERE o.execution_id=g.id AND o.state='outcome_unknown'),
            EXISTS(SELECT 1 FROM goal_criteria c WHERE c.goal_id=g.id)
            FROM goal_records g WHERE conversation_scope=? ORDER BY updated_at DESC,id LIMIT 20")
            .bind(&scope.0).fetch_all(&self.pool).await.map_err(error)?;
        rows.into_iter()
            .map(
                |(
                    id,
                    schema_version,
                    revision,
                    state,
                    objective,
                    reason,
                    evidence_count,
                    unresolved_operations,
                    model_criteria_saved,
                )| {
                    if schema_version != i64::from(GOAL_SCHEMA_VERSION) || revision < 0 {
                        return Err(error("unsupported or invalid goal record"));
                    }
                    let state =
                        serde_json::from_value(serde_json::Value::String(state)).map_err(error)?;
                    Ok(GoalStatus {
                        id,
                        schema_version: GOAL_SCHEMA_VERSION,
                        revision,
                        state,
                        objective,
                        reason,
                        evidence_count,
                        unresolved_operations,
                        model_criteria_saved,
                    })
                },
            )
            .collect()
    }

    /// Resolve and verify a bounded evidence snapshot within the same access domain.
    pub async fn goal_evidence(
        &self,
        scope: &ConversationScope,
        id: &str,
        hash: &str,
    ) -> Result<Option<ToolResultEvidence>, Temm1eError> {
        let document: Option<String> = sqlx::query_scalar(
            "SELECT e.document FROM goal_evidence e JOIN goal_records g ON g.id=e.goal_id
            WHERE g.conversation_scope=? AND g.id=? AND e.hash=?",
        )
        .bind(&scope.0)
        .bind(id)
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(error)?;
        document
            .map(|document| {
                if document.len() > 512 * 1024
                    || hex::encode(Sha256::digest(document.as_bytes())) != hash
                {
                    return Err(error("evidence integrity check failed"));
                }
                let evidence: ToolResultEvidence =
                    serde_json::from_str(&document).map_err(error)?;
                if evidence.schema_version != GOAL_SCHEMA_VERSION {
                    return Err(error("unsupported evidence schema"));
                }
                Ok(evidence)
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use temm1e_test_utils::{make_inbound_msg, make_session};

    #[tokio::test]
    async fn tool_evidence_is_bounded_immutable_scoped_and_integrity_checked() {
        let directory = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&directory.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(directory.path(), "cli", "room", "alice").unwrap();
        let foreign = ConversationScope::new(directory.path(), "cli", "room", "bob").unwrap();
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().into();
        session.channel = "cli".into();
        session.chat_id = "room".into();
        session.user_id = "alice".into();
        session.session_id = turn.epoch().into();
        let id = journal
            .begin(&make_inbound_msg("verify behavior"), &session)
            .await
            .unwrap();
        journal
            .intent(
                &id,
                "op1",
                "shell",
                &serde_json::json!({"command":"fixture"}),
                &[],
            )
            .await
            .unwrap();
        let uncertain = journal.goal_status(&scope).await.unwrap();
        assert_eq!(uncertain[0].unresolved_operations, 1);
        assert_eq!(uncertain[0].evidence_count, 0);
        let output = "🦀 test output\n".repeat(10000);
        journal.result(&id, "op1", &output, false).await.unwrap();
        assert!(journal
            .result(&id, "op1", "replace original", true)
            .await
            .is_err());
        let hash: String = sqlx::query_scalar("SELECT hash FROM goal_evidence WHERE goal_id=?")
            .bind(&id)
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        let evidence = journal
            .goal_evidence(&scope, &id, &hash)
            .await
            .unwrap()
            .unwrap();
        assert!(evidence.truncated);
        assert!(evidence.content.len() < 66000);
        assert_eq!(evidence.original_bytes, output.len());
        assert_eq!(
            evidence.original_sha256,
            hex::encode(Sha256::digest(output.as_bytes()))
        );
        assert_eq!(evidence.tool, "shell");
        assert!(!evidence.is_error);
        assert!(journal.goal_status(&foreign).await.unwrap().is_empty());
        assert!(journal
            .goal_evidence(&foreign, &id, &hash)
            .await
            .unwrap()
            .is_none());
        journal
            .finish(&id, "reply_returned", &[], Some("DONE"))
            .await
            .unwrap();
        turn.commit(&[]).await.unwrap();
        let status = journal.goal_status(&scope).await.unwrap();
        assert_eq!(status[0].state, GoalState::AwaitingEvidence);
        assert_eq!(status[0].revision, 1);
        assert_eq!(status[0].evidence_count, 1);
        assert_eq!(status[0].unresolved_operations, 0);
        let kinds: Vec<String> =
            sqlx::query_scalar("SELECT kind FROM goal_events WHERE goal_id=? ORDER BY sequence")
                .bind(&id)
                .fetch_all(&journal.pool)
                .await
                .unwrap();
        assert_eq!(
            kinds,
            [
                "admitted",
                "tool_intent",
                "tool_result_recorded",
                "reply_returned"
            ]
        );
        sqlx::query("UPDATE goal_evidence SET document='{}' WHERE goal_id=?")
            .bind(&id)
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(journal.goal_evidence(&scope, &id, &hash).await.is_err());
    }

    #[tokio::test]
    async fn concurrent_terminal_transitions_commit_one_goal_revision_and_event() {
        let directory = tempfile::tempdir().unwrap();
        let journal = ExecutionJournal::open(&directory.path().join("executions.db"))
            .await
            .unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().into();
        let id = journal
            .begin(&make_inbound_msg("task"), &session)
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            journal.finish(&id, "reply_returned", &[], Some("DONE")),
            journal.finish(&id, "interrupted", &[], None)
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let (revision, state): (i64, String) =
            sqlx::query_as("SELECT revision,state FROM goal_records WHERE id=?")
                .bind(&id)
                .fetch_one(&journal.pool)
                .await
                .unwrap();
        assert_eq!(revision, 1);
        assert!(["awaiting_evidence", "recovering"].contains(&state.as_str()));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM goal_events WHERE goal_id=?")
            .bind(&id)
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn future_schema_is_rejected_and_legacy_history_is_not_promoted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("executions.db");
        let journal = ExecutionJournal::open(&path).await.unwrap();
        sqlx::query("INSERT INTO executions(id,scope,inbound_id,goal,state,checkpoint,reply,created_at,updated_at) VALUES('legacy','scope','old','original goal','reply_returned','[]','DONE','then','then')")
            .execute(&journal.pool).await.unwrap();
        drop(journal);
        let journal = ExecutionJournal::open(&path).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM goal_records")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        sqlx::query("UPDATE goal_schema SET version=2")
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(ExecutionJournal::open(&path).await.is_err());
        let reply: String = sqlx::query_scalar("SELECT reply FROM executions WHERE id='legacy'")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(reply, "DONE");
        let version: i64 = sqlx::query_scalar("SELECT version FROM goal_schema")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(version, 2);
    }
}
