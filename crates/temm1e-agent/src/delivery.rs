//! Final-reply outbox. Sink acceptance, user visibility and goal completion are
//! different facts. An attempted delivery is never automatically replayed.
use crate::{conversation::ConversationScope, execution_journal::ExecutionJournal};
use futures::FutureExt;
use sha2::{Digest, Sha256};
use std::{future::Future, sync::Arc, time::Duration};
use temm1e_core::{
    private_file::PrivateFileLock,
    types::{error::Temm1eError, message::OutboundMessage},
};

pub(crate) const MAX_REPLY_BYTES: usize = 1024 * 1024;
fn error(message: impl std::fmt::Display) -> Temm1eError {
    Temm1eError::Channel(format!("Reply delivery: {message}"))
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct DeliveryRecord {
    pub id: String,
    pub epoch: String,
    pub revision: i64,
    pub state: String,
    pub updated_at: String,
}

/// The stable conversation lock remains owned until the attempt finishes or is
/// dropped. Pending/attempting rows survive process death independently of it.
pub struct DeliveryTicket {
    pub(crate) journal: Arc<ExecutionJournal>,
    pub(crate) id: String,
    pub(crate) message: OutboundMessage,
    pub(crate) _lock: PrivateFileLock,
}

impl DeliveryTicket {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Invoke the sink at most once after committing an attempt marker. Closure
    /// construction happens after admission, so no transport work can precede it.
    pub async fn deliver<F, Fut>(self, sink: F) -> Result<(), Temm1eError>
    where
        F: FnOnce(OutboundMessage) -> Fut,
        Fut: Future<Output = Result<(), Temm1eError>>,
    {
        self.deliver_with_timeout(sink, Duration::from_secs(30))
            .await
    }

    async fn deliver_with_timeout<F, Fut>(
        self,
        sink: F,
        timeout: Duration,
    ) -> Result<(), Temm1eError>
    where
        F: FnOnce(OutboundMessage) -> Fut,
        Fut: Future<Output = Result<(), Temm1eError>>,
    {
        let owner = uuid::Uuid::new_v4().to_string();
        let changed = sqlx::query("UPDATE delivery_outbox SET state='attempting',attempt_owner=?,updated_at=? WHERE id=? AND state='pending'")
            .bind(&owner).bind(chrono::Utc::now().to_rfc3339()).bind(&self.id)
            .execute(&self.journal.pool).await.map_err(error)?;
        if changed.rows_affected() != 1 {
            return Err(error(
                "already attempted; reconciliation is required instead of replay",
            ));
        }
        // Any timeout/error after admission is potentially a partial effect,
        // including when a channel splits one logical message into many parts.
        let result = tokio::time::timeout(
            timeout,
            std::panic::AssertUnwindSafe(async { sink(self.message.clone()).await }).catch_unwind(),
        )
        .await;
        let (state, detail) = match result {
            Ok(Ok(Ok(()))) => (
                "accepted_by_sink",
                "sink returned success; visibility and goal completion are not established",
            ),
            Ok(Ok(Err(_))) | Ok(Err(_)) => (
                "outcome_unknown",
                "sink returned an error after attempt admission; partial delivery is possible",
            ),
            Err(_) => (
                "outcome_unknown",
                "delivery deadline expired after attempt admission; partial delivery is possible",
            ),
        };
        // Store only static explanations; transport errors can contain secrets.
        let changed = sqlx::query("UPDATE delivery_outbox SET state=?,detail=?,updated_at=? WHERE id=? AND state='attempting' AND attempt_owner=?")
            .bind(state).bind(detail).bind(chrono::Utc::now().to_rfc3339())
            .bind(&self.id).bind(owner).execute(&self.journal.pool).await.map_err(|_| error("could not persist the sink outcome; do not replay the attempted delivery"))?;
        if changed.rows_affected() != 1 {
            return Err(error(
                "attempt ownership changed; outcome requires reconciliation",
            ));
        }
        if state == "accepted_by_sink" {
            Ok(())
        } else {
            Err(error(format!(
                "{} has an unknown outcome; inspect /delivery-status before taking further action",
                self.id
            )))
        }
    }
}

impl ExecutionJournal {
    pub async fn delivery_records(
        &self,
        scope: &ConversationScope,
    ) -> Result<Vec<DeliveryRecord>, Temm1eError> {
        sqlx::query_as("SELECT id,epoch,revision,state,updated_at FROM delivery_outbox WHERE scope=? ORDER BY rowid DESC LIMIT 100")
            .bind(&scope.0).fetch_all(&self.pool).await.map_err(error)
    }

    /// Read exact saved text for an authorized review; this is not redelivery
    /// admission and it does not change the transport state.
    pub async fn saved_reply(
        &self,
        scope: &ConversationScope,
        id: &str,
    ) -> Result<OutboundMessage, Temm1eError> {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT payload,payload_hash FROM delivery_outbox WHERE scope=? AND id=?",
        )
        .bind(&scope.0)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(error)?;
        let (payload, digest) =
            row.ok_or_else(|| error("no saved reply in this conversation with that ID"))?;
        if payload.len() > MAX_REPLY_BYTES
            || hex::encode(Sha256::digest(payload.as_bytes())) != digest
        {
            return Err(error("saved reply failed its size or integrity check"));
        }
        let message: OutboundMessage = serde_json::from_str(&payload).map_err(error)?;
        if message.chat_id != scope.destination()?.1 {
            return Err(error("saved reply destination does not match conversation"));
        }
        Ok(message)
    }

    /// Explicit resume of a never-attempted reply. Scope, integrity and local
    /// ownership are checked before exposing a ticket; uncertain attempts fail.
    pub async fn resume_pending_delivery(
        self: &Arc<Self>,
        scope: &ConversationScope,
        id: &str,
    ) -> Result<DeliveryTicket, Temm1eError> {
        let lock = self.conversation_lock(scope)?;
        let (payload, digest, state, epoch): (String, String, String, String) = sqlx::query_as(
            "SELECT payload,payload_hash,state,epoch FROM delivery_outbox WHERE scope=? AND id=?",
        )
        .bind(&scope.0)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(error)?
        .ok_or_else(|| error("no delivery in this conversation with that ID"))?;
        let current_epoch: Option<String> =
            sqlx::query_scalar("SELECT epoch FROM conversation_heads WHERE scope=?")
                .bind(&scope.0)
                .fetch_optional(&self.pool)
                .await
                .map_err(error)?;
        if current_epoch.as_deref() != Some(epoch.as_str()) {
            return Err(error("reply belongs to an earlier conversation; use /delivery-show to review it without sending a stale response"));
        }
        if state != "pending" {
            return Err(error(
                "delivery was already attempted or acknowledged; automatic replay is forbidden",
            ));
        }
        if payload.len() > MAX_REPLY_BYTES
            || hex::encode(Sha256::digest(payload.as_bytes())) != digest
        {
            return Err(error("saved reply failed its size or integrity check"));
        }
        let message: OutboundMessage = serde_json::from_str(&payload).map_err(error)?;
        if message.chat_id != scope.destination()?.1 {
            return Err(error("saved reply destination does not match conversation"));
        }
        Ok(DeliveryTicket {
            journal: self.clone(),
            id: id.into(),
            message,
            _lock: lock,
        })
    }

    /// Records the user's explicit statement that they have received/read this
    /// reply. This is user-reported receipt, not a fabricated platform receipt.
    pub async fn acknowledge_delivery(
        &self,
        scope: &ConversationScope,
        id: &str,
    ) -> Result<(), Temm1eError> {
        let _lock = self.conversation_lock(scope)?;
        let changed = sqlx::query("UPDATE delivery_outbox SET state='acknowledged_by_user',detail='user explicitly acknowledged receipt',updated_at=? WHERE scope=? AND id=? AND state IN ('pending','attempting','outcome_unknown')")
            .bind(chrono::Utc::now().to_rfc3339()).bind(&scope.0).bind(id).execute(&self.pool).await.map_err(error)?;
        if changed.rows_affected() != 1 {
            return Err(error("delivery is absent or already acknowledged"));
        }
        Ok(())
    }
}

/// Parse an authenticated owner's explicit request to send a never-attempted
/// saved reply. Entrypoints supply their own sink; this invokes no model/tool.
pub async fn prepare_resume_command(
    journal: &Arc<ExecutionJournal>,
    scope: &ConversationScope,
    text: &str,
) -> Result<Option<DeliveryTicket>, Temm1eError> {
    let args: Vec<_> = text.split_whitespace().collect();
    if args.first().copied() != Some("/delivery-resume") {
        return Ok(None);
    }
    if args.len() != 2 {
        return Err(error(
            "Usage: /delivery-resume <id> for a pending, never-attempted reply",
        ));
    }
    journal
        .resume_pending_delivery(scope, args[1])
        .await
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use temm1e_core::types::message::{ChatMessage, MessageContent, Role};
    async fn setup() -> (tempfile::TempDir, Arc<ExecutionJournal>, ConversationScope) {
        let directory = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&directory.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(directory.path(), "fixture", "chat", "owner").unwrap();
        (directory, journal, scope)
    }
    fn reply() -> OutboundMessage {
        OutboundMessage {
            chat_id: "chat".into(),
            text: "Verified fixture reply 🦀".into(),
            reply_to: None,
            parse_mode: None,
        }
    }
    fn history() -> Vec<ChatMessage> {
        vec![ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Keep this instruction.".into()),
        }]
    }
    async fn stage(journal: &Arc<ExecutionJournal>, scope: &ConversationScope) -> DeliveryTicket {
        journal
            .acquire_conversation(scope)
            .await
            .unwrap()
            .commit_with_reply(&history(), &reply())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn reply_and_history_commit_atomically_then_resume_without_provider_work() {
        let (_dir, journal, scope) = setup().await;
        let mut wrong = reply();
        wrong.chat_id = "someone-else".into();
        assert!(journal
            .acquire_conversation(&scope)
            .await
            .unwrap()
            .commit_with_reply(&history(), &wrong)
            .await
            .is_err());
        assert!(journal.delivery_records(&scope).await.unwrap().is_empty());
        let checkpoint: String = sqlx::query_scalar("SELECT checkpoint FROM conversation_heads")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(checkpoint, "[]");
        journal.new_conversation(&scope).await.unwrap();
        let ticket = stage(&journal, &scope).await;
        let id = ticket.id().to_string();
        assert!(journal.acquire_conversation(&scope).await.is_err());
        drop(ticket); // Crash before admission: pending is safe to explicitly resume.
        assert!(journal.acquire_conversation(&scope).await.is_err());
        let ticket = journal.resume_pending_delivery(&scope, &id).await.unwrap();
        ticket
            .deliver(|message| async move {
                assert_eq!(message.text, reply().text);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            journal.delivery_records(&scope).await.unwrap()[0].state,
            "accepted_by_sink"
        );
        assert!(journal.resume_pending_delivery(&scope, &id).await.is_err());
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        assert_eq!(turn.history().len(), 1);
        turn.commit(&history()).await.unwrap();
    }

    #[tokio::test]
    async fn archived_pending_reply_is_reviewable_but_cannot_interrupt_new_conversation() {
        let (_dir, journal, scope) = setup().await;
        let ticket = stage(&journal, &scope).await;
        let id = ticket.id().to_string();
        drop(ticket);
        journal.new_conversation(&scope).await.unwrap();
        assert!(journal.resume_pending_delivery(&scope, &id).await.is_err());
        assert_eq!(
            journal.saved_reply(&scope, &id).await.unwrap().text,
            reply().text
        );
        assert_eq!(
            journal.delivery_records(&scope).await.unwrap()[0].state,
            "pending"
        );
        journal
            .acquire_conversation(&scope)
            .await
            .unwrap()
            .commit(&[])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cancelled_after_effect_stays_unknown_and_cannot_be_replayed() {
        let (_dir, journal, scope) = setup().await;
        let ticket = stage(&journal, &scope).await;
        let id = ticket.id().to_string();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (started, notified) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(ticket.deliver(move |_| async move {
            observed.fetch_add(1, Ordering::SeqCst);
            let _ = started.send(());
            std::future::pending::<Result<(), Temm1eError>>().await
        }));
        notified.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(
            journal.delivery_records(&scope).await.unwrap()[0].state,
            "attempting"
        );
        assert!(journal.resume_pending_delivery(&scope, &id).await.is_err());
        assert!(journal.acquire_conversation(&scope).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        journal.acknowledge_delivery(&scope, &id).await.unwrap();
        assert_eq!(
            journal.delivery_records(&scope).await.unwrap()[0].state,
            "acknowledged_by_user"
        );
        journal
            .acquire_conversation(&scope)
            .await
            .unwrap()
            .commit(&history())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn errors_panics_and_timeouts_are_uncertain_without_leaking_errors() {
        let (_dir, journal, scope) = setup().await;
        let ticket = stage(&journal, &scope).await;
        assert!(ticket
            .deliver(|_| async { Err(Temm1eError::Channel("secret transport details".into())) })
            .await
            .is_err());
        let detail: String = sqlx::query_scalar("SELECT detail FROM delivery_outbox")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert!(!detail.contains("secret"));
        journal.new_conversation(&scope).await.unwrap();
        let ticket = stage(&journal, &scope).await;
        assert!(ticket
            .deliver(|_| async { panic!("fixture sink panic") })
            .await
            .is_err());
        journal.new_conversation(&scope).await.unwrap();
        let ticket = stage(&journal, &scope).await;
        assert!(ticket
            .deliver_with_timeout(
                |_| std::future::pending::<Result<(), Temm1eError>>(),
                Duration::from_millis(1)
            )
            .await
            .is_err());
        let records = journal.delivery_records(&scope).await.unwrap();
        assert_eq!(records.len(), 3);
        assert!(records
            .iter()
            .all(|record| record.state == "outcome_unknown"));
    }

    #[tokio::test]
    async fn only_one_resumer_and_scope_and_integrity_are_checked() {
        let (dir, journal, scope) = setup().await;
        let ticket = stage(&journal, &scope).await;
        let id = ticket.id().to_string();
        drop(ticket);
        let (first, second) = tokio::join!(
            journal.resume_pending_delivery(&scope, &id),
            journal.resume_pending_delivery(&scope, &id)
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        drop(first);
        drop(second);
        let other = ConversationScope::new(dir.path(), "fixture", "other-chat", "owner").unwrap();
        assert!(journal.resume_pending_delivery(&other, &id).await.is_err());
        sqlx::query("UPDATE delivery_outbox SET payload='corrupt' WHERE id=?")
            .bind(&id)
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(journal.resume_pending_delivery(&scope, &id).await.is_err());
    }

    #[tokio::test]
    async fn acknowledgement_write_failure_does_not_make_the_send_replayable() {
        let (_dir, journal, scope) = setup().await;
        let ticket = stage(&journal, &scope).await;
        let id = ticket.id().to_string();
        sqlx::query("CREATE TRIGGER reject_ack BEFORE UPDATE ON delivery_outbox WHEN NEW.state='accepted_by_sink' BEGIN SELECT RAISE(ABORT,'fixture acknowledgement write failure'); END")
            .execute(&journal.pool).await.unwrap();
        let calls = AtomicUsize::new(0);
        assert!(ticket
            .deliver(|_| async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            journal.delivery_records(&scope).await.unwrap()[0].state,
            "attempting"
        );
        assert!(journal.resume_pending_delivery(&scope, &id).await.is_err());
    }

    #[tokio::test]
    async fn oversized_reply_preserves_prior_history_without_a_pending_row() {
        let (_dir, journal, scope) = setup().await;
        let mut oversized = reply();
        oversized.text = "x".repeat(MAX_REPLY_BYTES);
        assert!(journal
            .acquire_conversation(&scope)
            .await
            .unwrap()
            .commit_with_reply(&history(), &oversized)
            .await
            .is_err());
        assert!(journal.delivery_records(&scope).await.unwrap().is_empty());
        let checkpoint: String = sqlx::query_scalar("SELECT checkpoint FROM conversation_heads")
            .fetch_one(&journal.pool)
            .await
            .unwrap();
        assert_eq!(checkpoint, "[]");
    }
}
