//! Canonical local conversation storage. A stable OS lock covers dispatch and
//! commit; a durable busy marker survives process death. It is never a license
//! to replay an interrupted tool. This lock protocol is for one local host.
use crate::execution_journal::{ExecutionJournal, ExecutionRecord};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use temm1e_core::{
    private_file::PrivateFileLock,
    types::{error::Temm1eError, message::ChatMessage},
};

fn error(e: impl std::fmt::Display) -> Temm1eError {
    Temm1eError::Internal(format!("Conversation: {e}"))
}

/// Access domains are chosen by the authenticated entrypoint, never the model.
/// A group channel deliberately supplies one shared domain for admitted members.
#[derive(Clone)]
pub struct ConversationScope(pub(crate) String);
impl ConversationScope {
    pub(crate) fn destination(&self) -> Result<(String, String), Temm1eError> {
        let (_, _, channel, chat, _): (String, std::path::PathBuf, String, String, String) =
            serde_json::from_str(&self.0).map_err(error)?;
        Ok((channel, chat))
    }

    pub fn new(
        workspace: &Path,
        channel: &str,
        chat: &str,
        access_domain: &str,
    ) -> Result<Self, Temm1eError> {
        if [channel, chat, access_domain].iter().any(|s| s.is_empty()) {
            return Err(error("scope identities must be nonempty"));
        }
        Ok(Self(
            serde_json::to_string(&(
                "conversation-v1",
                workspace.canonicalize().map_err(error)?,
                channel,
                chat,
                access_domain,
            ))
            .map_err(error)?,
        ))
    }
}

/// Dropping without commit deliberately leaves recovery required. In particular,
/// unwinding or cancelling a provider future must not erase uncertain effects.
pub struct ConversationTurn {
    journal: Arc<ExecutionJournal>,
    scope: ConversationScope,
    epoch: String,
    revision: i64,
    owner: String,
    history: Vec<ChatMessage>,
    _lock: PrivateFileLock,
}

/// A cursor is bound to a committed head; it never authorizes a turn or replay.
#[derive(Debug, Clone)]
pub struct ConversationCursor {
    scope: String,
    epoch: String,
    revision: i64,
    before: usize,
}

#[derive(Debug)]
pub struct ConversationPage {
    pub messages: Vec<ChatMessage>,
    pub older: Option<ConversationCursor>,
    pub older_messages: usize,
    pub recovery_required: bool,
    pub delivery_unresolved: bool,
}

impl ExecutionJournal {
    /// Read only a bounded page of the committed transcript. Does not acquire a
    /// turn, set busy state, acknowledge delivery, or expose another scope.
    pub async fn conversation_page(
        &self,
        scope: &ConversationScope,
        cursor: Option<&ConversationCursor>,
    ) -> Result<ConversationPage, Temm1eError> {
        let mut tx = self.pool.begin().await.map_err(error)?;
        let head: Option<(String, i64, String, Option<String>)> = sqlx::query_as(
            "SELECT epoch,revision,checkpoint,busy_owner FROM conversation_heads WHERE scope=?",
        )
        .bind(&scope.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(error)?;
        let Some((epoch, revision, checkpoint, busy)) = head else {
            if cursor.is_some() {
                return Err(error("history changed; reload the latest page"));
            }
            return Ok(ConversationPage {
                messages: vec![],
                older: None,
                older_messages: 0,
                recovery_required: false,
                delivery_unresolved: false,
            });
        };
        if cursor.is_some_and(|c| c.scope != scope.0 || c.epoch != epoch || c.revision != revision)
        {
            return Err(error(
                "history changed; use /history to reload the latest page",
            ));
        }
        if checkpoint.len() > 32 * 1024 * 1024 {
            return Err(error("oversized history checkpoint"));
        }
        let (selected, start) = if checkpoint.trim_start().starts_with('[') {
            let messages: Vec<ChatMessage> = serde_json::from_str(&checkpoint).map_err(error)?;
            if messages.len() > 100_000 {
                return Err(error("too many history messages"));
            }
            let end = cursor.map_or(messages.len(), |c| c.before);
            if end > messages.len() {
                return Err(error("invalid history cursor"));
            }
            let start = end.saturating_sub(100);
            (
                serde_json::to_string(&messages[start..end]).map_err(error)?,
                start,
            )
        } else {
            #[derive(serde::Deserialize, serde::Serialize)]
            #[serde(deny_unknown_fields)]
            struct Manifest {
                version: u32,
                message_ids: Vec<String>,
            }
            let mut manifest: Manifest = serde_json::from_str(&checkpoint).map_err(error)?;
            if manifest.version != 1 || manifest.message_ids.len() > 100_000 {
                return Err(error("invalid history manifest"));
            }
            let end = cursor.map_or(manifest.message_ids.len(), |c| c.before);
            if end > manifest.message_ids.len() {
                return Err(error("invalid history cursor"));
            }
            let start = end.saturating_sub(100);
            manifest.message_ids = manifest.message_ids[start..end].to_vec();
            (serde_json::to_string(&manifest).map_err(error)?, start)
        };
        let records = Self::hydrate_records_on(
            &mut tx,
            &scope.0,
            vec![ExecutionRecord {
                id: epoch.clone(),
                inbound_id: String::new(),
                goal: String::new(),
                state: String::new(),
                checkpoint: selected,
                updated_at: String::new(),
            }],
        )
        .await?;
        let messages = serde_json::from_str(&records[0].checkpoint).map_err(error)?;
        let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_outbox WHERE scope=? AND epoch=? AND state IN ('pending','attempting','outcome_unknown'))")
            .bind(&scope.0).bind(&epoch).fetch_one(&mut *tx).await.map_err(error)?;
        Ok(ConversationPage {
            messages,
            older: (start > 0).then_some(ConversationCursor {
                scope: scope.0.clone(),
                epoch,
                revision,
                before: start,
            }),
            older_messages: start,
            recovery_required: busy.is_some(),
            delivery_unresolved: unresolved,
        })
    }

    pub(crate) fn conversation_lock(
        &self,
        scope: &ConversationScope,
    ) -> Result<PrivateFileLock, Temm1eError> {
        let lock = self
            .path
            .with_extension("conversation-locks")
            .join(hex::encode(Sha256::digest(scope.0.as_bytes())));
        PrivateFileLock::try_exclusive(&lock)
            .map_err(error)?
            .ok_or_else(|| error("another process is using this conversation"))
    }

    /// Load the authoritative head under the same lock that will cover the turn.
    /// Legacy memory keys are intentionally neither inferred nor modified.
    pub async fn acquire_conversation(
        self: &Arc<Self>,
        scope: &ConversationScope,
    ) -> Result<ConversationTurn, Temm1eError> {
        let lock = self.conversation_lock(scope)?;
        let owner = uuid::Uuid::new_v4().to_string();
        let epoch = uuid::Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await.map_err(error)?;
        sqlx::query("INSERT OR IGNORE INTO conversation_heads(scope,epoch,revision,checkpoint) VALUES(?,?,0,'[]')")
            .bind(&scope.0).bind(epoch).execute(&mut *tx).await.map_err(error)?;
        let (epoch, revision, checkpoint, busy): (String, i64, String, Option<String>) =
            sqlx::query_as(
                "SELECT epoch,revision,checkpoint,busy_owner FROM conversation_heads WHERE scope=?",
            )
            .bind(&scope.0)
            .fetch_one(&mut *tx)
            .await
            .map_err(error)?;
        if busy.is_some() {
            return Err(error("interrupted conversation: use /session-recover to inspect evidence or /session-new to start separately; tools were not replayed"));
        }
        let delivery_pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_outbox WHERE scope=? AND epoch=? AND state IN ('pending','attempting','outcome_unknown'))")
            .bind(&scope.0).bind(&epoch).fetch_one(&mut *tx).await.map_err(error)?;
        if delivery_pending {
            return Err(error("a saved reply needs delivery reconciliation; inspect /delivery-status, or use /session-new to start separately"));
        }
        // Hydration must succeed before a read failure can create a busy marker.
        let records = Self::hydrate_records_on(
            &mut tx,
            &scope.0,
            vec![ExecutionRecord {
                id: epoch.clone(),
                inbound_id: String::new(),
                goal: String::new(),
                state: String::new(),
                checkpoint,
                updated_at: String::new(),
            }],
        )
        .await?;
        let history = serde_json::from_str(&records[0].checkpoint).map_err(error)?;
        sqlx::query("UPDATE conversation_heads SET busy_owner=? WHERE scope=?")
            .bind(&owner)
            .bind(&scope.0)
            .execute(&mut *tx)
            .await
            .map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(ConversationTurn {
            journal: self.clone(),
            scope: scope.clone(),
            epoch,
            revision,
            owner,
            history,
            _lock: lock,
        })
    }

    /// Inspect a crashed turn, then restore its exact checkpoint only after an
    /// explicit digest confirmation. This performs no provider/tool/delivery call.
    pub async fn recover_conversation(
        self: &Arc<Self>,
        scope: &ConversationScope,
        confirmation: Option<&str>,
    ) -> Result<String, Temm1eError> {
        let lock = self.conversation_lock(scope)?;
        let head: Option<(String, i64, String, Option<String>)> = sqlx::query_as(
            "SELECT epoch,revision,checkpoint,busy_owner FROM conversation_heads WHERE scope=?",
        )
        .bind(&scope.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(error)?;
        let Some((epoch, revision, checkpoint, busy)) = head else {
            return Ok("No saved conversation to recover.".into());
        };
        let Some(owner) = busy else {
            return Ok("This conversation has no interrupted turn.".into());
        };
        let base = self
            .hydrate_records(
                &scope.0,
                vec![ExecutionRecord {
                    id: epoch.clone(),
                    inbound_id: String::new(),
                    goal: String::new(),
                    state: String::new(),
                    checkpoint,
                    updated_at: String::new(),
                }],
            )
            .await?;
        let original: Vec<ChatMessage> =
            serde_json::from_str(&base[0].checkpoint).map_err(error)?;
        let latest: Option<(String, String)> = sqlx::query_as(
            "SELECT e.scope,e.checkpoint FROM executions e JOIN execution_conversations c ON c.execution_id=e.id WHERE c.epoch=? AND c.revision=? ORDER BY e.rowid DESC LIMIT 1")
            .bind(&epoch).bind(revision).fetch_optional(&self.pool).await.map_err(error)?;
        let mut recovered = if let Some((execution_scope, checkpoint)) = latest {
            let records = self
                .hydrate_records(
                    &execution_scope,
                    vec![ExecutionRecord {
                        id: epoch.clone(),
                        inbound_id: String::new(),
                        goal: String::new(),
                        state: String::new(),
                        checkpoint,
                        updated_at: String::new(),
                    }],
                )
                .await?;
            serde_json::from_str(&records[0].checkpoint).map_err(error)?
        } else {
            original.clone()
        };
        let operations: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT o.operation_id,o.tool,o.state FROM execution_operations o JOIN execution_conversations c ON c.execution_id=o.execution_id WHERE c.epoch=? AND c.revision=? ORDER BY o.execution_id,o.operation_id LIMIT 1001")
            .bind(&epoch).bind(revision).fetch_all(&self.pool).await.map_err(error)?;
        if operations.len() > 1000 {
            return Err(error("recovery operation list exceeds 1000; inspect archived evidence before continuation"));
        }
        let token = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&epoch, revision, &owner, &recovered, &operations))
                .map_err(error)?,
        ));
        let unknown = operations
            .iter()
            .filter(|(_, _, state)| state == "outcome_unknown")
            .count();
        let mut repaired = recovered.clone();
        crate::runtime::record_interrupted_tool_results(&mut repaired);
        let unresolved_native = if repaired.len() > recovered.len() {
            match &repaired.last().expect("repair appended a message").content {
                temm1e_core::types::message::MessageContent::Parts(parts) => parts.len(),
                _ => 0,
            }
        } else {
            0
        };
        if confirmation.is_none() {
            let details = operations
                .iter()
                .map(|(id, tool, state)| format!("{tool} [{id}]: {state}"))
                .collect::<Vec<_>>()
                .join("\n");
            return Ok(format!("Interrupted conversation {epoch}: {} checkpoint messages, {unknown} journal operations without a recorded result; {unresolved_native} native tool calls without a matching saved response.\n{details}\nA recorded tool result is not proof of the overall goal or reply delivery. To restore this evidence without replaying tools, enter /session-recover confirm {token}. Inspect external state before retrying any uncertain effect.", recovered.len()));
        }
        if confirmation != Some(token.as_str()) {
            return Err(error(
                "recovery evidence changed or confirmation is invalid; run /session-recover again",
            ));
        }
        recovered = repaired;
        recovered.push(ChatMessage {
            role: temm1e_core::types::message::Role::System,
            content: temm1e_core::types::message::MessageContent::Text(
                "The user explicitly restored evidence after an interrupted turn. No tools or replies were replayed by recovery. Unknown tool effects require inspection of external state before retrying; neither a returned reply nor this recovery establishes goal completion or successful delivery.".into()),
        });
        let turn = ConversationTurn {
            journal: self.clone(),
            scope: scope.clone(),
            epoch,
            revision,
            owner,
            history: original,
            _lock: lock,
        };
        turn.commit_boundary(&recovered, "recovery").await?;
        Ok(format!("Restored interrupted evidence: {unknown} journal operations and {unresolved_native} native calls still require outcome reconciliation. No tools or replies were replayed. Inspect uncertain effects before continuing."))
    }

    /// Explicit user action only. Archive the old boundary, retain all payloads
    /// and execution evidence, and start a distinct epoch with no old handoff.
    pub async fn new_conversation(&self, scope: &ConversationScope) -> Result<String, Temm1eError> {
        let _lock = self.conversation_lock(scope)?;
        let epoch = uuid::Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await.map_err(error)?;
        sqlx::query("INSERT INTO conversation_events(epoch,revision,kind,checkpoint,created_at) SELECT epoch,revision+1,'reset',checkpoint,? FROM conversation_heads WHERE scope=?")
            .bind(chrono::Utc::now().to_rfc3339()).bind(&scope.0).execute(&mut *tx).await.map_err(error)?;
        sqlx::query("INSERT INTO conversation_heads(scope,epoch,revision,checkpoint,busy_owner) VALUES(?,?,0,'[]',NULL) ON CONFLICT(scope) DO UPDATE SET epoch=excluded.epoch,revision=0,checkpoint='[]',busy_owner=NULL")
            .bind(&scope.0).bind(&epoch).execute(&mut *tx).await.map_err(error)?;
        tx.commit().await.map_err(error)?;
        Ok(epoch)
    }
}

impl ConversationTurn {
    pub fn epoch(&self) -> &str {
        &self.epoch
    }
    pub fn history(&self) -> &[ChatMessage] {
        &self.history
    }

    /// Commit a complete native history, preserving the previously saved prefix.
    /// Compaction changes provider requests, not canonical conversation evidence.
    pub async fn commit(self, history: &[ChatMessage]) -> Result<(), Temm1eError> {
        self.commit_boundary(history, "turn").await
    }

    pub async fn commit_with_reply(
        self,
        history: &[ChatMessage],
        reply: &temm1e_core::types::message::OutboundMessage,
    ) -> Result<crate::delivery::DeliveryTicket, Temm1eError> {
        self.commit_transaction(history, "turn", Some(reply))
            .await?
            .ok_or_else(|| error("reply transaction did not create a delivery ticket"))
    }

    async fn commit_boundary(self, history: &[ChatMessage], kind: &str) -> Result<(), Temm1eError> {
        self.commit_transaction(history, kind, None)
            .await
            .map(|_| ())
    }

    async fn commit_transaction(
        self,
        history: &[ChatMessage],
        kind: &str,
        reply: Option<&temm1e_core::types::message::OutboundMessage>,
    ) -> Result<Option<crate::delivery::DeliveryTicket>, Temm1eError> {
        if history.len() < self.history.len()
            || serde_json::to_vec(&history[..self.history.len()]).map_err(error)?
                != serde_json::to_vec(&self.history).map_err(error)?
        {
            return Err(error(
                "refusing to truncate or rewrite saved conversation history",
            ));
        }
        let mut tx = self.journal.pool.begin().await.map_err(error)?;
        let checkpoint = ExecutionJournal::store_history(&mut tx, &self.scope.0, history).await?;
        let changed = sqlx::query("UPDATE conversation_heads SET revision=revision+1,checkpoint=?,busy_owner=NULL WHERE scope=? AND epoch=? AND revision=? AND busy_owner=?")
            .bind(&checkpoint).bind(&self.scope.0).bind(&self.epoch).bind(self.revision).bind(&self.owner)
            .execute(&mut *tx).await.map_err(error)?;
        if changed.rows_affected() != 1 {
            return Err(error("stale conversation revision or owner"));
        }
        sqlx::query("INSERT INTO conversation_events(epoch,revision,kind,checkpoint,created_at) VALUES(?,?,?,?,?)")
            .bind(&self.epoch).bind(self.revision + 1).bind(kind).bind(checkpoint)
            .bind(chrono::Utc::now().to_rfc3339()).execute(&mut *tx).await.map_err(error)?;
        let delivery = if let Some(reply) = reply {
            let (channel, chat) = self.scope.destination()?;
            if reply.chat_id != chat {
                return Err(error("reply destination does not match conversation"));
            }
            let payload = serde_json::to_string(reply).map_err(error)?;
            if payload.len() > crate::delivery::MAX_REPLY_BYTES {
                return Err(error(
                    "final reply exceeds 1 MiB; prior history is preserved",
                ));
            }
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query("INSERT INTO delivery_outbox(id,scope,epoch,revision,channel,payload,payload_hash,state,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'pending',?,?)")
                .bind(&id).bind(&self.scope.0).bind(&self.epoch).bind(self.revision + 1)
                .bind(channel).bind(&payload).bind(hex::encode(Sha256::digest(payload.as_bytes())))
                .bind(&now).bind(&now).execute(&mut *tx).await.map_err(error)?;
            Some((id, reply.clone()))
        } else {
            None
        };
        tx.commit().await.map_err(error)?;
        Ok(
            delivery.map(|(id, message)| crate::delivery::DeliveryTicket {
                journal: self.journal.clone(),
                id,
                message,
                _lock: self._lock,
            }),
        )
    }
}

/// Owner command handling shared by authenticated entrypoints. No command is sent to a
/// model. The digest confirmation binds the displayed source to the import.
pub async fn handle_owner_command(
    journal: &Arc<ExecutionJournal>,
    scope: &ConversationScope,
    memory: &dyn temm1e_core::Memory,
    legacy_key: &str,
    text: &str,
) -> Result<Option<String>, Temm1eError> {
    let args: Vec<_> = text.split_whitespace().collect();
    match args.first().copied() {
        Some("/goal-status") => {
            if args.len() != 1 {
                return Ok(Some("Usage: /goal-status".into()));
            }
            let goals = journal.goal_status(scope).await?;
            if goals.is_empty() {
                return Ok(Some("No typed goal records in this conversation scope. Legacy history was not reinterpreted as verified goals.".into()));
            }
            let mut report = String::from(
                "Saved goal states (up to 20; objective previews up to 512 characters):\n",
            );
            for goal in goals {
                use std::fmt::Write;
                let objective: String = goal
                    .objective
                    .chars()
                    .flat_map(|c| {
                        if c.is_control() && c != '\n' && c != '\t' {
                            c.escape_default().collect::<Vec<_>>()
                        } else {
                            vec![c]
                        }
                    })
                    .collect();
                let criteria = if goal.model_criteria_saved {
                    "model criteria saved; coverage unverified"
                } else {
                    "no saved model criteria"
                };
                let _ = writeln!(report, "{} | {} | revision {} | {} evidence snapshots | {} uncertain operations | {}\n{}\n{}", goal.id, goal.state.as_str(), goal.revision, goal.evidence_count, goal.unresolved_operations, criteria, objective, goal.reason);
            }
            report.push_str("A returned/delivered reply or recorded tool output does not prove achievement. Running is the last saved state, not a liveness or replay guarantee.");
            Ok(Some(report))
        }
        Some("/delivery-status") => {
            if args.len() != 1 {
                return Ok(Some("Usage: /delivery-status".into()));
            }
            let records = journal.delivery_records(scope).await?;
            if records.is_empty() {
                return Ok(Some(
                    "No saved reply deliveries in this conversation scope.".into(),
                ));
            }
            let lines = records
                .iter()
                .map(|record| {
                    let label = match record.state.as_str() {
                        "pending" => "Saved, never sent",
                        "attempting" => "Sending or interrupted; receipt unconfirmed",
                        "outcome_unknown" => "Receipt unconfirmed",
                        "accepted_by_sink" => "Channel accepted it; display unconfirmed",
                        "acknowledged_by_user" => "You confirmed receipt",
                        _ => "Unrecognized delivery state; inspect before continuing",
                    };
                    format!(
                        "{} — {label} (conversation {}, revision {})",
                        record.id, record.epoch, record.revision
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(Some(format!("Most recent saved replies (up to 100):\n{lines}\nUse /delivery-show <id> to review a saved reply. /delivery-resume <id> sends only a never-attempted reply without running the model or tools. If you received/read it, /delivery-ack <id> records your confirmation. This does not establish that the overall task succeeded.")))
        }
        Some("/delivery-show") => {
            if !(2..=3).contains(&args.len()) {
                return Ok(Some("Usage: /delivery-show <id> [character offset]".into()));
            }
            let offset = args
                .get(2)
                .map(|value| value.parse::<usize>())
                .transpose()
                .map_err(error)?
                .unwrap_or(0);
            let reply = journal.saved_reply(scope, args[1]).await?;
            let count = reply.text.chars().count();
            if offset > count {
                return Err(error("saved reply offset is beyond the text"));
            }
            let text: String = reply.text.chars().skip(offset).take(4096).collect();
            let next = offset + text.chars().count();
            let continuation = if next < count {
                format!("\nNext page: /delivery-show {} {next}", args[1])
            } else {
                String::new()
            };
            Ok(Some(format!("Saved reply {} (review copy; original delivery state is unchanged):\n{text}{continuation}", args[1])))
        }
        Some("/delivery-ack") => {
            if args.len() != 2 {
                return Ok(Some("Use /delivery-ack <id> only to report that you received/read that saved reply.".into()));
            }
            journal.acknowledge_delivery(scope, args[1]).await?;
            Ok(Some("Recorded your acknowledgement. No platform receipt or goal completion was inferred.".into()))
        }
        Some("/session-new") => {
            if args.len() != 1 {
                return Ok(Some("Usage: /session-new".into()));
            }
            let epoch = journal.new_conversation(scope).await?;
            Ok(Some(format!("Started conversation {epoch}. Previous history and interrupted execution evidence are preserved. No previous tools were replayed.")))
        }
        Some("/session-recover") => {
            let confirmation = match args.as_slice() {
                [_] => None,
                [_, "confirm", token] => Some(*token),
                _ => {
                    return Ok(Some(
                        "Usage: /session-recover, then /session-recover confirm <digest>".into(),
                    ))
                }
            };
            journal
                .recover_conversation(scope, confirmation)
                .await
                .map(Some)
        }
        Some("/history-import") => {
            let Some(entry) = memory.get(legacy_key).await? else {
                return Ok(Some(format!("No legacy history at {legacy_key}.")));
            };
            if entry.content.len() > 32 * 1024 * 1024 {
                return Err(error("legacy history exceeds 32 MiB"));
            }
            let history: Vec<ChatMessage> = serde_json::from_str(&entry.content).map_err(error)?;
            if history.len() > 100_000 {
                return Err(error("legacy history exceeds 100000 messages"));
            }
            let digest = hex::encode(Sha256::digest(entry.content.as_bytes()));
            if args.len() == 1 {
                return Ok(Some(format!("Legacy source {legacy_key}: {} messages. Its original workspace is unknown. To copy into this empty conversation, enter /history-import confirm {digest}. The source will be preserved; no tools will be replayed.", history.len())));
            }
            if args.len() != 3 || args[1] != "confirm" || args[2] != digest {
                return Err(error("import confirmation does not match the current legacy source; run /history-import again"));
            }
            let turn = journal.acquire_conversation(scope).await?;
            let kind = format!("import:{digest}");
            let imported: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM conversation_events WHERE epoch=? AND kind=?)",
            )
            .bind(turn.epoch())
            .bind(&kind)
            .fetch_one(&journal.pool)
            .await
            .map_err(error)?;
            if imported {
                let unchanged = turn.history().to_vec();
                turn.commit(&unchanged).await?;
                return Ok(Some("This source was already imported into this conversation; history was not duplicated.".into()));
            }
            if !turn.history().is_empty() {
                // A rejected management command has dispatched no work. Release
                // its admission cleanly without losing existing history.
                let unchanged = turn.history().to_vec();
                turn.commit(&unchanged).await?;
                return Err(error("import requires an empty conversation; use /session-new to preserve and leave the current one"));
            }
            turn.commit_boundary(&history, &kind).await?;
            Ok(Some(format!(
                "Imported {} messages from {legacy_key}. The legacy source is unchanged.",
                history.len()
            )))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temm1e_core::types::message::{MessageContent, Role};
    fn messages(count: usize) -> Vec<ChatMessage> {
        (0..count)
            .map(|i| ChatMessage {
                role: Role::User,
                content: MessageContent::Text(format!("constraint {i}")),
            })
            .collect()
    }
    #[tokio::test]
    async fn transcript_pages_are_scoped_read_only_and_reject_stale_heads() {
        let directory = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&directory.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(directory.path(), "tui", "tui", "owner").unwrap();
        let other = ConversationScope::new(directory.path(), "tui", "tui", "other").unwrap();
        journal
            .acquire_conversation(&scope)
            .await
            .unwrap()
            .commit(&messages(250))
            .await
            .unwrap();
        let page = journal.conversation_page(&scope, None).await.unwrap();
        assert_eq!(page.messages.len(), 100);
        assert_eq!(page.older_messages, 150);
        assert_eq!(
            serde_json::to_value(&page.messages).unwrap(),
            serde_json::to_value(&messages(250)[150..]).unwrap()
        );
        assert!(!page.recovery_required && !page.delivery_unresolved);
        assert!(journal
            .conversation_page(&other, None)
            .await
            .unwrap()
            .messages
            .is_empty());
        assert!(journal
            .conversation_page(&other, page.older.as_ref())
            .await
            .is_err());
        let older = journal
            .conversation_page(&scope, page.older.as_ref())
            .await
            .unwrap();
        assert_eq!(older.older_messages, 50);
        assert_eq!(
            journal
                .conversation_page(&scope, older.older.as_ref())
                .await
                .unwrap()
                .messages
                .len(),
            50
        );
        // Reading did not acquire a turn or mark it busy.
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        assert!(
            journal
                .conversation_page(&scope, None)
                .await
                .unwrap()
                .recovery_required
        );
        turn.commit(&messages(251)).await.unwrap();
        assert!(journal
            .conversation_page(&scope, page.older.as_ref())
            .await
            .is_err());
        let current = journal.conversation_page(&scope, None).await.unwrap();
        assert_eq!(current.older_messages, 151);
    }

    #[tokio::test]
    async fn admission_does_not_wait_for_a_second_pool_connection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("executions.db");
        let mut journal = ExecutionJournal::open(&path).await.unwrap();
        journal.pool.close().await;
        journal.pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
        let journal = Arc::new(journal);
        let scope = ConversationScope::new(directory.path(), "fixture", "chat", "owner").unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            journal
                .acquire_conversation(&scope)
                .await
                .unwrap()
                .commit(&messages(3))
                .await
                .unwrap();
            let turn = journal.acquire_conversation(&scope).await.unwrap();
            assert_eq!(turn.history().len(), 3);
            turn.commit(&messages(4)).await.unwrap();
        })
        .await
        .expect("history hydration must reuse its admission transaction's connection");
    }

    #[tokio::test]
    async fn recovery_restores_crashed_intent_without_replaying_or_claiming_success() {
        use temm1e_core::types::message::ContentPart;
        use temm1e_test_utils::{make_inbound_msg, make_session};
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&dir.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(dir.path(), "test", "chat", "owner").unwrap();
        journal
            .acquire_conversation(&scope)
            .await
            .unwrap()
            .commit(&messages(2))
            .await
            .unwrap();
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        let mut session = make_session();
        session.workspace_path = dir.path().to_owned();
        session.session_id = turn.epoch().into();
        session.history = turn.history().to_vec();
        let input = make_inbound_msg("write one artifact");
        let execution = journal.begin(&input, &session).await.unwrap();
        session.history.push(ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id: "native-call".into(),
                thought_signature: None,
                name: "file_write".into(),
                input: serde_json::json!({"path":"artifact"}),
            }]),
        });
        journal
            .intent(
                &execution,
                "operation",
                "file_write",
                &serde_json::json!({}),
                &session.history,
            )
            .await
            .unwrap();
        drop(turn);
        assert!(journal.acquire_conversation(&scope).await.is_err());
        let preview = journal.recover_conversation(&scope, None).await.unwrap();
        assert!(preview.contains("1 journal operations"));
        assert!(preview.contains("1 native tool calls"));
        assert!(journal
            .recover_conversation(&scope, Some("wrong"))
            .await
            .is_err());
        let token = preview
            .split("/session-recover confirm ")
            .nth(1)
            .unwrap()
            .split('.')
            .next()
            .unwrap();
        journal
            .recover_conversation(&scope, Some(token))
            .await
            .unwrap();
        let restored = journal.acquire_conversation(&scope).await.unwrap();
        assert_eq!(restored.history().len(), 5);
        let serialized = serde_json::to_string(restored.history()).unwrap();
        assert!(serialized.contains("Outcome unknown"));
        assert!(serialized.contains("constraint 0"));
        let unchanged = restored.history().to_vec();
        restored.commit(&unchanged).await.unwrap();
        let operations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM execution_operations")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(operations, 1);
        assert!(journal.begin(&input, &session).await.is_err());
    }

    #[tokio::test]
    async fn legacy_import_is_confirmed_preserved_and_idempotent() {
        use temm1e_core::Memory;
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&dir.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(dir.path(), "cli", "cli", "local-owner").unwrap();
        let memory = temm1e_memory::SqliteMemory::new("sqlite::memory:")
            .await
            .unwrap();
        let raw = serde_json::to_string(&messages(240)).unwrap();
        memory
            .store(temm1e_core::MemoryEntry {
                id: "chat_history:cli".into(),
                content: raw.clone(),
                metadata: serde_json::json!({}),
                timestamp: chrono::Utc::now(),
                session_id: None,
                entry_type: temm1e_core::MemoryEntryType::Conversation,
            })
            .await
            .unwrap();
        let preview = handle_owner_command(
            &journal,
            &scope,
            &memory,
            "chat_history:cli",
            "/history-import",
        )
        .await
        .unwrap()
        .unwrap();
        assert!(preview.contains("240 messages"));
        assert!(handle_owner_command(
            &journal,
            &scope,
            &memory,
            "chat_history:cli",
            "/history-import confirm wrong"
        )
        .await
        .is_err());
        let command = format!(
            "/history-import confirm {}",
            hex::encode(Sha256::digest(raw.as_bytes()))
        );
        handle_owner_command(&journal, &scope, &memory, "chat_history:cli", &command)
            .await
            .unwrap();
        let repeat = handle_owner_command(&journal, &scope, &memory, "chat_history:cli", &command)
            .await
            .unwrap()
            .unwrap();
        assert!(repeat.contains("already imported"));
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        assert_eq!(turn.history().len(), 240);
        turn.commit(&messages(241)).await.unwrap();
        assert_eq!(
            memory
                .get("chat_history:cli")
                .await
                .unwrap()
                .unwrap()
                .content,
            raw
        );
        // Another source cannot overwrite an existing conversation.
        let mut entry = memory.get("chat_history:cli").await.unwrap().unwrap();
        entry.content = serde_json::to_string(&messages(2)).unwrap();
        let changed = format!(
            "/history-import confirm {}",
            hex::encode(Sha256::digest(entry.content.as_bytes()))
        );
        memory.store(entry).await.unwrap();
        assert!(
            handle_owner_command(&journal, &scope, &memory, "chat_history:cli", &changed)
                .await
                .is_err()
        );
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        assert_eq!(turn.history().len(), 241);
        turn.commit(&messages(241)).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn canonical_workspace_aliases_share_a_lock_but_other_workspaces_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let alias = dir.path().join("alias");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&dir.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope =
            |path: &Path| ConversationScope::new(path, "cli", "cli", "local-owner").unwrap();
        let turn = journal.acquire_conversation(&scope(&first)).await.unwrap();
        assert!(journal.acquire_conversation(&scope(&alias)).await.is_err());
        journal
            .acquire_conversation(&scope(&second))
            .await
            .unwrap()
            .commit(&[])
            .await
            .unwrap();
        turn.commit(&[]).await.unwrap();
    }

    #[tokio::test]
    async fn stale_owner_cannot_commit_or_clear_new_owner() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&dir.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(dir.path(), "cli", "cli", "local").unwrap();
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        sqlx::query("UPDATE conversation_heads SET busy_owner='replacement',revision=revision+1 WHERE scope=?")
            .bind(&scope.0).execute(&journal.pool).await.unwrap();
        assert!(turn.commit(&messages(1)).await.is_err());
        let state: (String, String) =
            sqlx::query_as("SELECT busy_owner,checkpoint FROM conversation_heads WHERE scope=?")
                .bind(&scope.0)
                .fetch_one(&journal.pool)
                .await
                .unwrap();
        assert_eq!(state, ("replacement".into(), "[]".into()));
    }

    #[tokio::test]
    async fn restart_keeps_more_than_200_messages_and_new_epoch_archives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("executions.db");
        let scope = ConversationScope::new(dir.path(), "cli", "cli", "local").unwrap();
        let journal = Arc::new(ExecutionJournal::open(&path).await.unwrap());
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        let epoch = turn.epoch().to_string();
        turn.commit(&messages(250)).await.unwrap();
        journal.pool.close().await;
        let journal = Arc::new(ExecutionJournal::open(&path).await.unwrap());
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        assert_eq!(turn.epoch(), epoch);
        assert_eq!(
            serde_json::to_value(turn.history()).unwrap(),
            serde_json::to_value(messages(250)).unwrap()
        );
        turn.commit(&messages(251)).await.unwrap();
        let new_epoch = journal.new_conversation(&scope).await.unwrap();
        assert_ne!(new_epoch, epoch);
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        assert!(turn.history().is_empty());
        turn.commit(&[]).await.unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM conversation_events WHERE epoch=?")
                .bind(epoch)
                .fetch_one(&journal.pool)
                .await
                .unwrap();
        assert_eq!(count, 3);
    }
    #[tokio::test]
    async fn lock_and_crash_marker_prevent_second_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("executions.db");
        let scope = ConversationScope::new(dir.path(), "cli", "cli", "local").unwrap();
        let first = Arc::new(ExecutionJournal::open(&path).await.unwrap());
        let second = Arc::new(ExecutionJournal::open(&path).await.unwrap());
        let turn = first.acquire_conversation(&scope).await.unwrap();
        assert!(second.acquire_conversation(&scope).await.is_err());
        assert!(second.new_conversation(&scope).await.is_err());
        drop(turn); // Same durable state as process death, with the OS lock gone.
        assert!(second.acquire_conversation(&scope).await.is_err());
        second.new_conversation(&scope).await.unwrap();
        second
            .acquire_conversation(&scope)
            .await
            .unwrap()
            .commit(&[])
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn scope_separation_corruption_and_prefix_rewrite_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&dir.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let first = ConversationScope::new(dir.path(), "a:b", "c", "group").unwrap();
        let second = ConversationScope::new(dir.path(), "a", "b:c", "group").unwrap();
        journal
            .acquire_conversation(&first)
            .await
            .unwrap()
            .commit(&messages(2))
            .await
            .unwrap();
        let turn = journal.acquire_conversation(&second).await.unwrap();
        assert!(turn.history().is_empty());
        turn.commit(&[]).await.unwrap();
        let turn = journal.acquire_conversation(&first).await.unwrap();
        assert!(turn.commit(&messages(1)).await.is_err());
        let stored: String =
            sqlx::query_scalar("SELECT checkpoint FROM conversation_heads WHERE scope=?")
                .bind(&first.0)
                .fetch_one(&journal.pool)
                .await
                .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stored).unwrap()["message_ids"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        journal.new_conversation(&first).await.unwrap();
        journal
            .acquire_conversation(&first)
            .await
            .unwrap()
            .commit(&messages(2))
            .await
            .unwrap();
        sqlx::query("DELETE FROM execution_history_payloads WHERE scope=?")
            .bind(&first.0)
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(journal.acquire_conversation(&first).await.is_err());
        let busy: Option<String> =
            sqlx::query_scalar("SELECT busy_owner FROM conversation_heads WHERE scope=?")
                .bind(&first.0)
                .fetch_one(&journal.pool)
                .await
                .unwrap();
        assert!(busy.is_none());
    }
}
