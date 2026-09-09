//! Immutable model-proposed criteria, distinct from proven user-goal coverage.
use crate::{conversation::ConversationScope, execution_journal::ExecutionJournal};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use temm1e_core::types::{error::Temm1eError, session::SessionContext};
use temm1e_witness::{oath::hash_oath, types::Oath};

const MAX_CRITERIA_BYTES: usize = 2 * 1024 * 1024;
fn error(e: impl std::fmt::Display) -> Temm1eError {
    Temm1eError::Internal(format!("Goal criteria: {e}"))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriteriaOrigin {
    ModelProposed,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriteriaCoverage {
    Unverified,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalCriteria {
    pub schema_version: u32,
    pub origin: CriteriaOrigin,
    pub coverage: CriteriaCoverage,
    pub workspace: PathBuf,
    pub oath: Oath,
}

impl ExecutionJournal {
    /// Freeze the initial proposal before effects. Private runtime authority,
    /// exact admitted identity/objective and revision CAS; no overwrite API.
    pub(crate) async fn record_goal_criteria(
        &self,
        id: &str,
        session: &SessionContext,
        oath: &Oath,
        expected_revision: i64,
    ) -> Result<String, Temm1eError> {
        if expected_revision < 0
            || oath.root_goal_id != id
            || oath.session_id != session.session_id
            || oath.sealed_hash != hash_oath(oath)
            || oath.postconditions.is_empty()
            || oath.postconditions.len() > 128
            || oath.preconditions.len() > 128
        {
            return Err(error("invalid or mismatched sealed proposal"));
        }
        let workspace = session.workspace_path.canonicalize().map_err(error)?;
        let snapshot = GoalCriteria {
            schema_version: 1,
            origin: CriteriaOrigin::ModelProposed,
            coverage: CriteriaCoverage::Unverified,
            workspace,
            oath: oath.clone(),
        };
        let document = serde_json::to_string(&snapshot).map_err(error)?;
        if document.len() > MAX_CRITERIA_BYTES {
            return Err(error("proposal exceeds 2 MiB; original goal preserved"));
        }
        let hash = hex::encode(Sha256::digest(document.as_bytes()));
        let scope = Self::scope(session)?;
        let principal = serde_json::to_string(&(&session.user_id, session.role)).map_err(error)?;
        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await.map_err(error)?;
        // A scoped epoch, if present, must also match. Embedded legacy callers
        // retain their exact execution scope without invented conversation IDs.
        let updated = sqlx::query("UPDATE goal_records SET revision=revision+1,updated_at=?
            WHERE id=? AND revision=? AND state='running' AND objective=? AND scope=? AND principal=?
            AND EXISTS(SELECT 1 FROM executions e WHERE e.id=goal_records.id AND e.state='running')
            AND NOT EXISTS(SELECT 1 FROM execution_conversations ec WHERE ec.execution_id=goal_records.id AND ec.epoch<>?)")
            .bind(&now).bind(id).bind(expected_revision).bind(&oath.goal).bind(scope).bind(principal)
            .bind(&session.session_id).execute(&mut *tx).await.map_err(error)?;
        if updated.rows_affected() != 1 {
            return Err(error(
                "stale revision, terminal execution or admission binding mismatch",
            ));
        }
        sqlx::query("INSERT INTO goal_criteria(goal_id,hash,document,created_at) VALUES(?,?,?,?)")
            .bind(id)
            .bind(&hash)
            .bind(document)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(error)?;
        Self::goal_event(
            &mut tx,
            id,
            "model_criteria_frozen",
            serde_json::json!({
                "criteria_hash":hash, "oath_hash":oath.sealed_hash,
                "coverage":"unverified", "postcondition_count":oath.postconditions.len()
            }),
        )
        .await?;
        tx.commit().await.map_err(error)?;
        Ok(hash)
    }

    /// Scoped inspection after restart. This is a proposal, not evidence that
    /// its predicates ran or that they cover the complete original request.
    pub async fn goal_criteria(
        &self,
        scope: &ConversationScope,
        id: &str,
    ) -> Result<Option<GoalCriteria>, Temm1eError> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT c.hash,c.document,g.objective FROM goal_criteria c JOIN goal_records g ON g.id=c.goal_id
             WHERE g.conversation_scope=? AND g.id=?")
            .bind(&scope.0).bind(id).fetch_optional(&self.pool).await.map_err(error)?;
        row.map(|(hash, document, objective)| {
            if document.len() > MAX_CRITERIA_BYTES
                || hex::encode(Sha256::digest(document.as_bytes())) != hash
            {
                return Err(error("proposal integrity check failed"));
            }
            let saved: GoalCriteria = serde_json::from_str(&document).map_err(error)?;
            if saved.schema_version != 1
                || saved.oath.root_goal_id != id
                || saved.oath.goal != objective
                || saved.oath.sealed_hash != hash_oath(&saved.oath)
            {
                return Err(error("unsupported or mismatched proposal"));
            }
            Ok(saved)
        })
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use temm1e_core::types::rbac::Role;
    use temm1e_test_utils::{make_inbound_msg, make_session};
    use temm1e_witness::types::Predicate;

    async fn fixture(
        directory: &std::path::Path,
    ) -> (
        Arc<ExecutionJournal>,
        SessionContext,
        ConversationScope,
        String,
        Oath,
    ) {
        let journal = Arc::new(
            ExecutionJournal::open(&directory.join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(directory, "test", "room", "alice").unwrap();
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        let mut session = make_session();
        session.workspace_path = directory.into();
        session.channel = "test".into();
        session.chat_id = "room".into();
        session.user_id = "alice".into();
        session.session_id = turn.epoch().into();
        let id = journal
            .begin(&make_inbound_msg("original objective"), &session)
            .await
            .unwrap();
        let mut oath = Oath::draft("set", &id, &session.session_id, "original objective")
            .with_postcondition(Predicate::DirectoryExists { path: ".".into() });
        oath.sealed_hash = hash_oath(&oath);
        (journal, session, scope, id, oath)
    }

    #[tokio::test]
    async fn frozen_proposal_reopens_scoped_and_detects_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath) = fixture(directory.path()).await;
        journal
            .record_goal_criteria(&id, &session, &oath, 0)
            .await
            .unwrap();
        // Second proposal must not overwrite even with the now-current revision.
        assert!(journal
            .record_goal_criteria(&id, &session, &oath, 1)
            .await
            .is_err());
        assert_eq!(journal.goal_status(&scope).await.unwrap()[0].revision, 1);
        let reopened = ExecutionJournal::open(&directory.path().join("executions.db"))
            .await
            .unwrap();
        let saved = reopened.goal_criteria(&scope, &id).await.unwrap().unwrap();
        assert_eq!(saved.oath.sealed_hash, oath.sealed_hash);
        assert_eq!(saved.workspace, directory.path().canonicalize().unwrap());
        let foreign = ConversationScope::new(directory.path(), "test", "room", "bob").unwrap();
        assert!(reopened
            .goal_criteria(&foreign, &id)
            .await
            .unwrap()
            .is_none());
        sqlx::query("UPDATE goal_criteria SET document='{}' WHERE goal_id=?")
            .bind(&id)
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(reopened.goal_criteria(&scope, &id).await.is_err());
        assert!(reopened
            .goal_criteria(&foreign, &id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn admission_binding_rejects_changed_objective_identity_workspace_epoch_and_seal() {
        let directory = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath) = fixture(directory.path()).await;
        for variant in 0..7 {
            let mut changed_session = session.clone();
            let mut changed_oath = oath.clone();
            match variant {
                0 => changed_oath.goal = "weakened objective".into(),
                1 => changed_session.user_id = "bob".into(),
                2 => changed_session.role = Role::User,
                3 => changed_session.workspace_path = other.path().into(),
                4 => {
                    changed_session.session_id = "other-epoch".into();
                    changed_oath.session_id = "other-epoch".into();
                }
                5 => changed_oath.root_goal_id = "other-goal".into(),
                _ => {}
            }
            changed_oath.sealed_hash = hash_oath(&changed_oath);
            if variant == 6 {
                changed_oath.sealed_hash = "0".repeat(64);
            }
            assert!(
                journal
                    .record_goal_criteria(&id, &changed_session, &changed_oath, 0)
                    .await
                    .is_err(),
                "variant {variant}"
            );
        }
        assert_eq!(journal.goal_status(&scope).await.unwrap()[0].revision, 0);
        assert!(journal.goal_criteria(&scope, &id).await.unwrap().is_none());
        journal
            .record_goal_criteria(&id, &session, &oath, 0)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn one_concurrent_proposal_wins_and_terminal_execution_rejects_mutation() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath) = fixture(directory.path()).await;
        let (first, second) = tokio::join!(
            journal.record_goal_criteria(&id, &session, &oath, 0),
            journal.record_goal_criteria(&id, &session, &oath, 0)
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert_eq!(journal.goal_status(&scope).await.unwrap()[0].revision, 1);
        journal
            .finish(&id, "reply_returned", &[], Some("DONE"))
            .await
            .unwrap();
        assert!(journal
            .record_goal_criteria(&id, &session, &oath, 2)
            .await
            .is_err());
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM goal_events WHERE goal_id=? AND kind='model_criteria_frozen'",
        )
        .bind(&id)
        .fetch_one(&journal.pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn oversized_or_empty_proposal_rolls_back_without_losing_the_original_goal() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, mut oath) = fixture(directory.path()).await;
        oath.postconditions.clear();
        oath.sealed_hash = hash_oath(&oath);
        assert!(journal
            .record_goal_criteria(&id, &session, &oath, 0)
            .await
            .is_err());
        oath.postconditions.push(Predicate::DirectoryExists {
            path: "x".repeat(MAX_CRITERIA_BYTES).into(),
        });
        oath.sealed_hash = hash_oath(&oath);
        assert!(journal
            .record_goal_criteria(&id, &session, &oath, 0)
            .await
            .is_err());
        let status = journal.goal_status(&scope).await.unwrap();
        assert_eq!(status[0].revision, 0);
        assert_eq!(status[0].objective, "original objective");
        assert!(journal.goal_criteria(&scope, &id).await.unwrap().is_none());
    }
}
