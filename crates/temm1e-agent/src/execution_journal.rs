//! Durable execution evidence. A returned reply is not proof that a goal was
//! achieved. Unfinished operation rows require reconciliation, never blind replay.
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::{path::Path, str::FromStr, time::Duration};
use temm1e_core::types::{
    error::Temm1eError,
    message::{ChatMessage, InboundMessage},
    session::SessionContext,
};

pub struct ExecutionJournal {
    pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExecutionRecord {
    pub id: String,
    pub inbound_id: String,
    pub goal: String,
    pub state: String,
    pub checkpoint: String,
    pub updated_at: String,
}

fn error(e: impl std::fmt::Display) -> Temm1eError {
    Temm1eError::Internal(format!("Execution journal: {e}"))
}

impl ExecutionJournal {
    pub async fn open_profile() -> Result<Self, Temm1eError> {
        let directory = temm1e_core::config::data_dir();
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).map_err(error)?;
        Self::open(&directory.join("executions.db")).await
    }

    pub async fn open(path: &Path) -> Result<Self, Temm1eError> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(error(e)),
        }
        let options = SqliteConnectOptions::from_str("sqlite:")
            .map_err(error)?
            .filename(path)
            .busy_timeout(Duration::from_secs(5))
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Full);
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(options)
            .await
            .map_err(error)?;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS executions (
            id TEXT PRIMARY KEY, scope TEXT NOT NULL, inbound_id TEXT NOT NULL,
            goal TEXT NOT NULL, state TEXT NOT NULL, checkpoint TEXT NOT NULL,
            reply TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS executions_scope_state ON executions(scope, state);
            CREATE TABLE IF NOT EXISTS execution_operations (
            execution_id TEXT NOT NULL REFERENCES executions(id), operation_id TEXT NOT NULL,
            tool TEXT NOT NULL, arguments TEXT NOT NULL, state TEXT NOT NULL,
            output TEXT, is_error INTEGER, updated_at TEXT NOT NULL,
            PRIMARY KEY (execution_id, operation_id));",
        )
        .execute(&pool)
        .await
        .map_err(error)?;
        Ok(Self { pool })
    }

    /// Scope uses a structured tuple, so separator characters cannot alias users.
    fn scope(session: &SessionContext) -> Result<String, Temm1eError> {
        let workspace = session.workspace_path.canonicalize().map_err(error)?;
        serde_json::to_string(&(
            &session.channel,
            &session.user_id,
            &session.chat_id,
            workspace,
        ))
        .map_err(error)
    }

    pub async fn begin(
        &self,
        msg: &InboundMessage,
        session: &SessionContext,
    ) -> Result<String, Temm1eError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO executions (id,scope,inbound_id,goal,state,checkpoint,created_at,updated_at) VALUES (?,?,?,?,'running',?,?,?)")
            .bind(&id).bind(Self::scope(session)?).bind(&msg.id).bind(msg.text.as_deref().unwrap_or(""))
            .bind(serde_json::to_string(&session.history).map_err(error)?).bind(&now).bind(&now)
            .execute(&self.pool).await.map_err(error)?;
        Ok(id)
    }

    /// Commit intent and the corresponding history in the same transaction,
    /// before dispatching any tool. Absence of a result means unknown outcome.
    pub async fn intent(
        &self,
        execution: &str,
        operation: &str,
        tool: &str,
        arguments: &serde_json::Value,
        history: &[ChatMessage],
    ) -> Result<(), Temm1eError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await.map_err(error)?;
        sqlx::query("INSERT INTO execution_operations (execution_id,operation_id,tool,arguments,state,updated_at) VALUES (?,?,?,?,'outcome_unknown',?)")
            .bind(execution).bind(operation).bind(tool).bind(arguments.to_string()).bind(&now)
            .execute(&mut *tx).await.map_err(error)?;
        let updated = sqlx::query(
            "UPDATE executions SET checkpoint=?, updated_at=? WHERE id=? AND state='running'",
        )
        .bind(serde_json::to_string(history).map_err(error)?)
        .bind(&now)
        .bind(execution)
        .execute(&mut *tx)
        .await
        .map_err(error)?;
        if updated.rows_affected() != 1 {
            return Err(error("execution is no longer running; intent rejected"));
        }
        tx.commit().await.map_err(error)?;
        Ok(())
    }

    pub async fn result(
        &self,
        execution: &str,
        operation: &str,
        output: &str,
        is_error: bool,
    ) -> Result<(), Temm1eError> {
        let mut end = output.len().min(65_536);
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        let output = if end < output.len() {
            format!(
                "{}\n[Journal output truncated; {} bytes total]",
                &output[..end],
                output.len()
            )
        } else {
            output.to_owned()
        };
        let updated = sqlx::query("UPDATE execution_operations SET state='result_recorded', output=?, is_error=?, updated_at=? WHERE execution_id=? AND operation_id=? AND state='outcome_unknown'")
            .bind(output).bind(is_error).bind(chrono::Utc::now().to_rfc3339()).bind(execution).bind(operation)
            .execute(&self.pool).await.map_err(error)?;
        if updated.rows_affected() != 1 {
            return Err(error(
                "operation missing or already resolved; result rejected",
            ));
        }
        Ok(())
    }

    pub async fn finish(
        &self,
        id: &str,
        state: &str,
        history: &[ChatMessage],
        reply: Option<&str>,
    ) -> Result<(), Temm1eError> {
        if !["reply_returned", "interrupted", "failed"].contains(&state) {
            return Err(error("invalid execution state"));
        }
        let updated = sqlx::query(
            "UPDATE executions SET state=?, checkpoint=?, reply=?, updated_at=? WHERE id=? AND state='running'",
        )
        .bind(state)
        .bind(serde_json::to_string(history).map_err(error)?)
        .bind(reply)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(error)?;
        if updated.rows_affected() != 1 {
            return Err(error(
                "execution missing or already terminal; transition rejected",
            ));
        }
        Ok(())
    }

    /// Read recovery candidates only in the caller's user/chat/workspace scope.
    /// This does not authorize replay and does not mark a goal complete.
    pub async fn unfinished(
        &self,
        session: &SessionContext,
    ) -> Result<Vec<ExecutionRecord>, Temm1eError> {
        sqlx::query_as("SELECT id,inbound_id,goal,state,checkpoint,updated_at FROM executions WHERE scope=? AND state IN ('running','interrupted','failed') ORDER BY updated_at DESC LIMIT 100")
            .bind(Self::scope(session)?).fetch_all(&self.pool).await.map_err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temm1e_test_utils::{make_inbound_msg, make_session};

    #[tokio::test]
    async fn restart_preserves_intent_and_scopes_recovery_without_claiming_completion() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("executions.db");
        let mut session = make_session();
        session.workspace_path = directory.path().to_owned();
        let journal = ExecutionJournal::open(&path).await.unwrap();
        let id = journal
            .begin(&make_inbound_msg("Create artifact"), &session)
            .await
            .unwrap();
        journal
            .intent(
                &id,
                "operation",
                "write",
                &serde_json::json!({"path":"artifact"}),
                &session.history,
            )
            .await
            .unwrap();
        journal.pool.close().await;
        let journal = ExecutionJournal::open(&path).await.unwrap();
        let records = journal.unfinished(&session).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, id);
        let state: String =
            sqlx::query_scalar("SELECT state FROM execution_operations WHERE execution_id=?")
                .bind(&id)
                .fetch_one(&journal.pool)
                .await
                .unwrap();
        assert_eq!(state, "outcome_unknown");
        let mut other = make_session();
        other.workspace_path = directory.path().to_owned();
        other.user_id = "another-user".into();
        assert!(journal.unfinished(&other).await.unwrap().is_empty());
        journal
            .result(&id, "operation", "file exists", false)
            .await
            .unwrap();
        journal
            .finish(
                &id,
                "reply_returned",
                &session.history,
                Some("Here is the result"),
            )
            .await
            .unwrap();
        assert!(journal.unfinished(&session).await.unwrap().is_empty());
        let state: String = sqlx::query_scalar("SELECT state FROM executions WHERE id=?")
            .bind(&id)
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(state, "reply_returned"); // explicitly not goal completion
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn duplicate_operation_intent_is_rejected_before_reexecution() {
        let directory = tempfile::tempdir().unwrap();
        let journal = ExecutionJournal::open(&directory.path().join("executions.db"))
            .await
            .unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().to_owned();
        let id = journal
            .begin(&make_inbound_msg("Test"), &session)
            .await
            .unwrap();
        for expected_ok in [true, false] {
            assert_eq!(
                journal
                    .intent(&id, "same", "shell", &serde_json::json!({}), &[])
                    .await
                    .is_ok(),
                expected_ok
            );
        }
    }

    #[tokio::test]
    async fn terminal_execution_cannot_accept_intents_or_be_rewritten() {
        let directory = tempfile::tempdir().unwrap();
        let journal = ExecutionJournal::open(&directory.path().join("executions.db"))
            .await
            .unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().to_owned();
        let id = journal
            .begin(&make_inbound_msg("Test"), &session)
            .await
            .unwrap();
        journal
            .intent(&id, "first", "shell", &serde_json::json!({}), &[])
            .await
            .unwrap();
        journal
            .result(&id, "first", "observed", false)
            .await
            .unwrap();
        assert!(journal
            .result(&id, "first", "replacement", false)
            .await
            .is_err());
        assert!(journal
            .result(&id, "missing", "fabricated", false)
            .await
            .is_err());
        journal.finish(&id, "interrupted", &[], None).await.unwrap();
        assert!(journal
            .finish(&id, "reply_returned", &[], Some("done"))
            .await
            .is_err());
        assert!(journal
            .intent(&id, "late", "shell", &serde_json::json!({}), &[])
            .await
            .is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM execution_operations WHERE execution_id=? AND operation_id='late'")
            .bind(&id).fetch_one(&journal.pool).await.unwrap();
        assert_eq!(count, 0, "rejected intent must roll back its insert");
    }
}
