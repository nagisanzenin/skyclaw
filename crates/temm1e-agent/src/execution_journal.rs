//! Durable execution evidence. A returned reply is not proof that a goal was
//! achieved. Unfinished operation rows require reconciliation, never blind replay.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::collections::HashMap;
use std::{path::Path, str::FromStr, time::Duration};
use temm1e_core::types::{
    error::Temm1eError,
    message::{ChatMessage, InboundMessage},
    session::SessionContext,
};

pub struct ExecutionJournal {
    pub(crate) pool: SqlitePool,
    pub(crate) path: std::path::PathBuf,
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredHandoff {
    schema_version: u32,
    source_hash: String,
    high_water: usize,
    source_ids: Vec<String>,
    summary: crate::compaction::Summary,
}

const MAX_HISTORY_BYTES: usize = 32 * 1024 * 1024;
const MAX_HISTORY_MESSAGES: usize = 100_000;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredHistory {
    version: u32,
    message_ids: Vec<String>,
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
            CREATE INDEX IF NOT EXISTS executions_scope_inbound ON executions(scope, inbound_id);
            CREATE TABLE IF NOT EXISTS inbound_claims (
                scope TEXT NOT NULL, inbound_id TEXT NOT NULL, execution_id TEXT NOT NULL,
                PRIMARY KEY(scope,inbound_id));
            CREATE TABLE IF NOT EXISTS execution_operations (
            execution_id TEXT NOT NULL REFERENCES executions(id), operation_id TEXT NOT NULL,
            tool TEXT NOT NULL, arguments TEXT NOT NULL, state TEXT NOT NULL,
            output TEXT, is_error INTEGER, updated_at TEXT NOT NULL,
            PRIMARY KEY (execution_id, operation_id));
            CREATE TABLE IF NOT EXISTS execution_history_payloads (
                scope TEXT NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL,
                PRIMARY KEY(scope,id));
            CREATE TABLE IF NOT EXISTS execution_conversations (
                execution_id TEXT PRIMARY KEY, epoch TEXT NOT NULL, revision INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS execution_conversations_epoch_revision ON execution_conversations(epoch,revision);
            CREATE TABLE IF NOT EXISTS delivery_outbox (
                id TEXT PRIMARY KEY, scope TEXT NOT NULL, epoch TEXT NOT NULL,
                revision INTEGER NOT NULL, channel TEXT NOT NULL,
                payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('pending','attempting','accepted_by_sink','outcome_unknown','acknowledged_by_user')),
                attempt_owner TEXT, detail TEXT,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                UNIQUE(epoch,revision));
            CREATE INDEX IF NOT EXISTS delivery_outbox_scope_epoch ON delivery_outbox(scope,epoch,state);
            CREATE TABLE IF NOT EXISTS conversation_heads (
                scope TEXT PRIMARY KEY, epoch TEXT NOT NULL, revision INTEGER NOT NULL,
                checkpoint TEXT NOT NULL, busy_owner TEXT);
            CREATE UNIQUE INDEX IF NOT EXISTS conversation_heads_epoch ON conversation_heads(epoch);
            CREATE TABLE IF NOT EXISTS conversation_events (
                epoch TEXT NOT NULL, revision INTEGER NOT NULL, kind TEXT NOT NULL,
                checkpoint TEXT NOT NULL, created_at TEXT NOT NULL,
                PRIMARY KEY(epoch,revision));
            CREATE TABLE IF NOT EXISTS context_heads (
                scope TEXT PRIMARY KEY, generation INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS context_handoffs (
                scope TEXT NOT NULL, generation INTEGER NOT NULL, document TEXT NOT NULL,
                created_at TEXT NOT NULL, PRIMARY KEY(scope,generation));
            CREATE TABLE IF NOT EXISTS context_source_messages (
                scope TEXT NOT NULL, id TEXT NOT NULL, sequence INTEGER NOT NULL,
                message TEXT NOT NULL, PRIMARY KEY(scope,id));",
        )
        .execute(&pool)
        .await
        .map_err(error)?;
        Ok(Self {
            pool,
            path: path.canonicalize().map_err(error)?,
        })
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

    pub(crate) async fn store_history(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        scope: &str,
        history: &[ChatMessage],
    ) -> Result<String, Temm1eError> {
        if history.len() > MAX_HISTORY_MESSAGES {
            return Err(error(
                "history exceeds 100000 messages; prior evidence is preserved",
            ));
        }
        let raw = serde_json::to_vec(history).map_err(error)?;
        if raw.len() > MAX_HISTORY_BYTES {
            return Err(error("history exceeds 32 MiB; prior evidence is preserved"));
        }
        let mut payloads = HashMap::new();
        let mut message_ids = Vec::with_capacity(history.len());
        for message in history {
            let payload = serde_json::to_string(message).map_err(error)?;
            let id = hex::encode(Sha256::digest(payload.as_bytes()));
            message_ids.push(id.clone());
            payloads.insert(id, payload);
        }
        let rows: Vec<_> = payloads
            .into_iter()
            .map(|(id, payload)| serde_json::json!({"id":id,"payload":payload}))
            .collect();
        sqlx::query("INSERT OR IGNORE INTO execution_history_payloads(scope,id,payload) SELECT ?,json_extract(value,'$.id'),json_extract(value,'$.payload') FROM json_each(?)")
            .bind(scope).bind(serde_json::to_string(&rows).map_err(error)?).execute(&mut **tx).await.map_err(error)?;
        serde_json::to_string(&StoredHistory {
            version: 1,
            message_ids,
        })
        .map_err(error)
    }

    pub(crate) async fn hydrate_records(
        &self,
        scope: &str,
        records: Vec<ExecutionRecord>,
    ) -> Result<Vec<ExecutionRecord>, Temm1eError> {
        let mut connection = self.pool.acquire().await.map_err(error)?;
        Self::hydrate_records_on(&mut connection, scope, records).await
    }

    pub(crate) async fn hydrate_records_on(
        connection: &mut sqlx::SqliteConnection,
        scope: &str,
        mut records: Vec<ExecutionRecord>,
    ) -> Result<Vec<ExecutionRecord>, Temm1eError> {
        let mut total_bytes = 0usize;
        for record in &mut records {
            if record.checkpoint.len() > MAX_HISTORY_BYTES {
                return Err(error("oversized history checkpoint"));
            }
            if record.checkpoint.trim_start().starts_with('[') {
                // Older inline checkpoints remain readable and are not rewritten.
                let _: Vec<ChatMessage> =
                    serde_json::from_str(&record.checkpoint).map_err(error)?;
            } else {
                let stored: StoredHistory =
                    serde_json::from_str(&record.checkpoint).map_err(error)?;
                if stored.message_ids.len() > MAX_HISTORY_MESSAGES {
                    return Err(error("history manifest exceeds message limit"));
                }
                if stored.version != 1 {
                    return Err(error("unsupported history checkpoint version"));
                }
                let rows: Vec<(String, String)> = sqlx::query_as("SELECT id,payload FROM execution_history_payloads WHERE scope=? AND id IN (SELECT value FROM json_each(?))")
                    .bind(scope).bind(serde_json::to_string(&stored.message_ids).map_err(error)?).fetch_all(&mut *connection).await.map_err(error)?;
                let mut messages = HashMap::new();
                for (id, payload) in rows {
                    if hex::encode(Sha256::digest(payload.as_bytes())) != id {
                        return Err(error("history payload integrity check failed"));
                    }
                    messages.insert(id, payload);
                }
                let mut raw = String::from("[");
                for id in stored.message_ids {
                    let payload = messages
                        .get(&id)
                        .ok_or_else(|| error("missing scoped history payload"))?;
                    if raw.len() > 1 {
                        raw.push(',');
                    }
                    if raw.len().saturating_add(payload.len()).saturating_add(1) > MAX_HISTORY_BYTES
                    {
                        return Err(error("hydrated history exceeds 32 MiB"));
                    }
                    raw.push_str(payload);
                }
                raw.push(']');
                let _: Vec<ChatMessage> = serde_json::from_str(&raw).map_err(error)?;
                record.checkpoint = raw;
            }
            total_bytes = total_bytes.saturating_add(record.checkpoint.len());
            if total_bytes > MAX_HISTORY_BYTES {
                return Err(error(
                    "recovery results exceed 32 MiB; request a specific inbound message",
                ));
            }
        }
        Ok(records)
    }

    pub async fn begin(
        &self,
        msg: &InboundMessage,
        session: &SessionContext,
    ) -> Result<String, Temm1eError> {
        if msg.id.is_empty() {
            return Err(error("inbound message requires a stable nonempty ID"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let scope = Self::scope(session)?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut transaction = self.pool.begin().await.map_err(error)?;
        // Claim first to serialize admissions before any provider/tool work.
        let claimed = sqlx::query("INSERT INTO inbound_claims(scope,inbound_id,execution_id) VALUES(?,?,?) ON CONFLICT(scope,inbound_id) DO NOTHING")
            .bind(&scope).bind(&msg.id).bind(&id).execute(&mut *transaction).await.map_err(error)?;
        if claimed.rows_affected() != 1 {
            return Err(error("inbound message already admitted; reconcile its existing execution instead of replaying it"));
        }
        // Preserve protection for records written before claim-table migration,
        // without deleting or rewriting historical duplicate execution evidence.
        let prior: Option<String> =
            sqlx::query_scalar("SELECT id FROM executions WHERE scope=? AND inbound_id=? LIMIT 1")
                .bind(&scope)
                .bind(&msg.id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(error)?;
        if let Some(prior) = prior {
            return Err(error(format!(
                "inbound message has prior execution {prior}; reconciliation is required"
            )));
        }
        let checkpoint = Self::store_history(&mut transaction, &scope, &session.history).await?;
        sqlx::query("INSERT INTO executions (id,scope,inbound_id,goal,state,checkpoint,created_at,updated_at) VALUES (?,?,?,?,'running',?,?,?)")
            .bind(&id).bind(scope).bind(&msg.id).bind(msg.text.as_deref().unwrap_or(""))
            .bind(checkpoint).bind(&now).bind(&now)
            .execute(&mut *transaction).await.map_err(error)?;
        // Only a canonical, currently admitted epoch can be linked. Legacy
        // string session IDs are never guessed into a conversation.
        sqlx::query("INSERT INTO execution_conversations(execution_id,epoch,revision) SELECT ?,epoch,revision FROM conversation_heads WHERE epoch=? AND busy_owner IS NOT NULL")
            .bind(&id).bind(&session.session_id).execute(&mut *transaction).await.map_err(error)?;
        transaction.commit().await.map_err(error)?;
        Ok(id)
    }

    /// Look up evidence for reconciliation. This does not authorize retry or delivery.
    pub async fn for_inbound(
        &self,
        session: &SessionContext,
        inbound_id: &str,
    ) -> Result<Vec<ExecutionRecord>, Temm1eError> {
        let scope = Self::scope(session)?;
        let records = sqlx::query_as("SELECT id,inbound_id,goal,state,checkpoint,updated_at FROM executions WHERE scope=? AND inbound_id=? ORDER BY created_at,id")
            .bind(&scope).bind(inbound_id).fetch_all(&self.pool).await.map_err(error)?;
        self.hydrate_records(&scope, records).await
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
        let scope: String =
            sqlx::query_scalar("SELECT scope FROM executions WHERE id=? AND state='running'")
                .bind(execution)
                .fetch_one(&mut *tx)
                .await
                .map_err(error)?;
        let checkpoint = Self::store_history(&mut tx, &scope, history).await?;
        sqlx::query("INSERT INTO execution_operations (execution_id,operation_id,tool,arguments,state,updated_at) VALUES (?,?,?,?,'outcome_unknown',?)")
            .bind(execution).bind(operation).bind(tool).bind(arguments.to_string()).bind(&now)
            .execute(&mut *tx).await.map_err(error)?;
        let updated = sqlx::query(
            "UPDATE executions SET checkpoint=?, updated_at=? WHERE id=? AND state='running'",
        )
        .bind(checkpoint)
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
        let mut tx = self.pool.begin().await.map_err(error)?;
        let scope: String =
            sqlx::query_scalar("SELECT scope FROM executions WHERE id=? AND state='running'")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(error)?;
        let checkpoint = Self::store_history(&mut tx, &scope, history).await?;
        let updated = sqlx::query(
            "UPDATE executions SET state=?, checkpoint=?, reply=?, updated_at=? WHERE id=? AND state='running'",
        )
        .bind(state)
        .bind(checkpoint)
        .bind(reply)
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(error)?;
        if updated.rows_affected() != 1 {
            return Err(error(
                "execution missing or already terminal; transition rejected",
            ));
        }
        tx.commit().await.map_err(error)?;
        Ok(())
    }

    fn context_scope(session: &SessionContext) -> Result<String, Temm1eError> {
        serde_json::to_string(&(Self::scope(session)?, &session.session_id)).map_err(error)
    }

    /// Immutable generations: concurrent compaction must compare-and-swap the
    /// observed head. Never replace a newer handoff with a stale summary.
    pub async fn save_handoff(
        &self,
        session: &SessionContext,
        expected: i64,
        handoff: &crate::compaction::Handoff,
    ) -> Result<i64, Temm1eError> {
        let raw: Vec<_> = handoff.sources.iter().map(|s| s.message.clone()).collect();
        handoff.validate(&raw)?;
        let generation = expected
            .checked_add(1)
            .ok_or_else(|| error("context generation overflow"))?;
        let scope = Self::context_scope(session)?;
        let mut tx = self.pool.begin().await.map_err(error)?;
        sqlx::query("INSERT OR IGNORE INTO context_heads(scope,generation) VALUES (?,0)")
            .bind(&scope)
            .execute(&mut *tx)
            .await
            .map_err(error)?;
        let changed =
            sqlx::query("UPDATE context_heads SET generation=? WHERE scope=? AND generation=?")
                .bind(generation)
                .bind(&scope)
                .bind(expected)
                .execute(&mut *tx)
                .await
                .map_err(error)?;
        if changed.rows_affected() != 1 {
            return Err(error(
                "stale compaction generation; raw history remains unchanged",
            ));
        }
        // Content-addressed immutable source rows avoid copying the entire raw
        // prefix into every successive handoff generation.
        for source in &handoff.sources {
            sqlx::query("INSERT OR IGNORE INTO context_source_messages(scope,id,sequence,message) VALUES (?,?,?,?)")
                .bind(&scope).bind(&source.id).bind(i64::try_from(source.sequence).map_err(error)?)
                .bind(serde_json::to_string(&source.message).map_err(error)?).execute(&mut *tx).await.map_err(error)?;
        }
        let stored = StoredHandoff {
            schema_version: handoff.schema_version,
            source_hash: handoff.source_hash.clone(),
            high_water: handoff.high_water,
            source_ids: handoff.sources.iter().map(|s| s.id.clone()).collect(),
            summary: handoff.summary.clone(),
        };
        sqlx::query(
            "INSERT INTO context_handoffs(scope,generation,document,created_at) VALUES (?,?,?,?)",
        )
        .bind(&scope)
        .bind(generation)
        .bind(serde_json::to_string(&stored).map_err(error)?)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(generation)
    }

    pub async fn load_handoff(
        &self,
        session: &SessionContext,
    ) -> Result<Option<(i64, crate::compaction::Handoff)>, Temm1eError> {
        let record: Option<(i64,String)> = sqlx::query_as("SELECT h.generation,d.document FROM context_heads h JOIN context_handoffs d ON h.scope=d.scope AND h.generation=d.generation WHERE h.scope=?")
            .bind(Self::context_scope(session)?).fetch_optional(&self.pool).await.map_err(error)?;
        let Some((generation, raw)) = record else {
            return Ok(None);
        };
        let document: serde_json::Value = serde_json::from_str(&raw).map_err(error)?;
        let handoff = if document.get("sources").is_some() {
            // Read the earlier development checkpoint's inline format without
            // rewriting its immutable evidence or breaking existing profiles.
            serde_json::from_value::<crate::compaction::Handoff>(document).map_err(error)?
        } else {
            let stored: StoredHandoff = serde_json::from_value(document).map_err(error)?;
            let rows: Vec<(i64,String,String)> = sqlx::query_as("SELECT sequence,id,message FROM context_source_messages WHERE scope=? AND id IN (SELECT value FROM json_each(?)) ORDER BY sequence")
                .bind(Self::context_scope(session)?).bind(serde_json::to_string(&stored.source_ids).map_err(error)?)
                .fetch_all(&self.pool).await.map_err(error)?;
            if rows
                .iter()
                .map(|(_, id, _)| id)
                .ne(stored.source_ids.iter())
            {
                return Err(error("missing or reordered context source rows"));
            }
            let sources = rows
                .into_iter()
                .map(|(sequence, id, message)| {
                    Ok(crate::compaction::SourceMessage {
                        sequence: usize::try_from(sequence).map_err(error)?,
                        id,
                        message: serde_json::from_str(&message).map_err(error)?,
                    })
                })
                .collect::<Result<Vec<_>, Temm1eError>>()?;
            crate::compaction::Handoff {
                schema_version: stored.schema_version,
                source_hash: stored.source_hash,
                high_water: stored.high_water,
                sources,
                summary: stored.summary,
            }
        };
        let source: Vec<_> = handoff.sources.iter().map(|s| s.message.clone()).collect();
        handoff.validate(&source)?;
        Ok(Some((generation, handoff)))
    }

    /// Read recovery candidates only in the caller's user/chat/workspace scope.
    /// This does not authorize replay and does not mark a goal complete.
    pub async fn unfinished(
        &self,
        session: &SessionContext,
    ) -> Result<Vec<ExecutionRecord>, Temm1eError> {
        let scope = Self::scope(session)?;
        let records = sqlx::query_as("SELECT id,inbound_id,goal,state,checkpoint,updated_at FROM executions WHERE scope=? AND state IN ('running','interrupted','failed') ORDER BY updated_at DESC LIMIT 100")
            .bind(&scope).fetch_all(&self.pool).await.map_err(error)?;
        self.hydrate_records(&scope, records).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temm1e_test_utils::{make_inbound_msg, make_session};

    #[tokio::test]
    async fn oversized_history_rolls_back_admission_without_discarding_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let journal = ExecutionJournal::open(&directory.path().join("executions.db"))
            .await
            .unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().to_owned();
        let message = make_inbound_msg("continue");
        session.history = vec![
            ChatMessage {
                role: temm1e_core::types::message::Role::User,
                content: temm1e_core::types::message::MessageContent::Text(String::new())
            };
            MAX_HISTORY_MESSAGES + 1
        ];
        assert!(journal.begin(&message, &session).await.is_err());
        assert_eq!(session.history.len(), MAX_HISTORY_MESSAGES + 1);
        assert!(journal
            .for_inbound(&session, &message.id)
            .await
            .unwrap()
            .is_empty());
        session.history.clear();
        assert!(journal.begin(&message, &session).await.is_ok());
    }

    #[tokio::test]
    async fn checkpoints_deduplicate_payloads_preserve_order_and_detect_corruption() {
        use temm1e_core::types::message::{MessageContent, Role};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("executions.db");
        let journal = ExecutionJournal::open(&path).await.unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().to_owned();
        let message = ChatMessage {
            role: Role::User,
            content: MessageContent::Text("original instruction".repeat(1000)),
        };
        session.history = vec![message.clone(), message.clone()];
        let mut inputs = Vec::new();
        for _ in 0..3 {
            let input = make_inbound_msg("continue");
            let id = journal.begin(&input, &session).await.unwrap();
            journal
                .intent(&id, "op", "read", &serde_json::json!({}), &session.history)
                .await
                .unwrap();
            journal
                .finish(&id, "reply_returned", &session.history, Some("returned"))
                .await
                .unwrap();
            inputs.push(input);
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM execution_history_payloads")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let stored: String = sqlx::query_scalar("SELECT checkpoint FROM executions LIMIT 1")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert!(!stored.contains("original instruction"));
        journal.pool.close().await;
        let journal = ExecutionJournal::open(&path).await.unwrap();
        let records = journal.for_inbound(&session, &inputs[0].id).await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&records[0].checkpoint).unwrap(),
            serde_json::to_value(&session.history).unwrap()
        );
        // Read old inline checkpoints without rewriting them.
        sqlx::query("UPDATE executions SET checkpoint=? WHERE inbound_id=?")
            .bind(serde_json::to_string(&session.history).unwrap())
            .bind(&inputs[0].id)
            .execute(&journal.pool)
            .await
            .unwrap();
        assert_eq!(
            journal.for_inbound(&session, &inputs[0].id).await.unwrap()[0].checkpoint,
            serde_json::to_string(&session.history).unwrap()
        );
        sqlx::query("UPDATE execution_history_payloads SET payload='{}'")
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(journal
            .for_inbound(&session, &inputs[1].id)
            .await
            .unwrap_err()
            .to_string()
            .contains("integrity"));
    }

    #[tokio::test]
    async fn concurrent_and_restarted_admissions_do_not_replay_an_inbound_message() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("executions.db");
        let first = ExecutionJournal::open(&path).await.unwrap();
        let second = ExecutionJournal::open(&path).await.unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().to_owned();
        let message = make_inbound_msg("perform one effect");
        let (a, b) = tokio::join!(
            first.begin(&message, &session),
            second.begin(&message, &session)
        );
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        let execution = first
            .for_inbound(&session, &message.id)
            .await
            .unwrap()
            .remove(0);
        first
            .finish(&execution.id, "reply_returned", &[], Some("returned"))
            .await
            .unwrap();
        first.pool.close().await;
        second.pool.close().await;
        let restarted = ExecutionJournal::open(&path).await.unwrap();
        assert!(restarted.begin(&message, &session).await.is_err());
        assert_eq!(
            restarted.for_inbound(&session, &message.id).await.unwrap()[0].state,
            "reply_returned"
        );
        // Legacy evidence without a claim row must still prevent replay.
        sqlx::query("DELETE FROM inbound_claims")
            .execute(&restarted.pool)
            .await
            .unwrap();
        assert!(restarted.begin(&message, &session).await.is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM executions")
            .fetch_one(&restarted.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        sqlx::query("INSERT INTO executions(id,scope,inbound_id,goal,state,checkpoint,created_at,updated_at) SELECT 'legacy-duplicate',scope,inbound_id,goal,state,checkpoint,created_at,updated_at FROM executions WHERE id=?")
            .bind(&execution.id).execute(&restarted.pool).await.unwrap();
        assert_eq!(
            restarted
                .for_inbound(&session, &message.id)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(restarted.begin(&message, &session).await.is_err());

        let mut other = session.clone();
        other.user_id = "another-user".into();
        assert!(restarted
            .for_inbound(&other, &message.id)
            .await
            .unwrap()
            .is_empty());
        assert!(restarted.begin(&message, &other).await.is_ok());
    }

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
