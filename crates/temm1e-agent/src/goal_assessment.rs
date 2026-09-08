//! Scoped immutable evaluator observations, not proof of full request coverage.
use crate::{
    conversation::ConversationScope,
    execution_journal::ExecutionJournal,
    goal_criteria::{CriteriaCoverage, GoalCriteria},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::PathBuf};
use temm1e_core::types::{
    error::Temm1eError,
    goal::{assess_required, Assessment, AssessmentOutcome, Criterion, EvaluatorKind},
    session::SessionContext,
};
use temm1e_witness::{
    oath::hash_oath,
    types::{Oath, Predicate, PredicateResult, Verdict, VerdictOutcome},
};

const MAX_DOCUMENT: usize = 4 * 1024 * 1024;
fn error(e: impl std::fmt::Display) -> Temm1eError {
    Temm1eError::Internal(format!("Goal assessment: {e}"))
}
fn digest(document: &str) -> String {
    hex::encode(Sha256::digest(document.as_bytes()))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    EvaluatorReport,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluatorObservation {
    pub schema_version: u32,
    pub kind: ObservationKind,
    pub criterion_id: String,
    pub evaluator_version: String,
    pub workspace: PathBuf,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    /// The evaluator's narrow report, not a snapshot of the inspected artifact.
    pub result: PredicateResult,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashedObservation {
    pub hash: String,
    pub observation: EvaluatorObservation,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalAssessment {
    pub schema_version: u32,
    pub criteria_hash: String,
    pub coverage: CriteriaCoverage,
    pub declared_outcome: AssessmentOutcome,
    pub observations: Vec<HashedObservation>,
}

fn criteria_document(
    document: &str,
    hash: &str,
    id: &str,
    objective: &str,
) -> Result<GoalCriteria, Temm1eError> {
    if document.len() > 2 * 1024 * 1024 || digest(document) != hash {
        return Err(error("criteria integrity check failed"));
    }
    let saved: GoalCriteria = serde_json::from_str(document).map_err(error)?;
    if saved.schema_version != 1
        || saved.oath.root_goal_id != id
        || saved.oath.goal != objective
        || saved.oath.sealed_hash != hash_oath(&saved.oath)
    {
        return Err(error("invalid frozen criteria"));
    }
    Ok(saved)
}

fn declared_outcome(
    criteria: &GoalCriteria,
    observations: &[HashedObservation],
) -> Result<AssessmentOutcome, Temm1eError> {
    if observations.len() != criteria.oath.postconditions.len() || observations.len() > 128 {
        return Err(error("observations do not match the frozen set"));
    }
    let mut requirements = Vec::new();
    let mut assessments = Vec::new();
    let mut hashes = HashSet::new();
    for (index, (predicate, entry)) in criteria
        .oath
        .postconditions
        .iter()
        .zip(observations)
        .enumerate()
    {
        let observation = &entry.observation;
        let result = &observation.result;
        let advisory = matches!(
            predicate,
            Predicate::AspectVerifier { advisory: true, .. } | Predicate::AdversarialJudge { .. }
        );
        let id = format!("{}:{index}", criteria.oath.sealed_hash);
        let tier = predicate.tier();
        let version = if tier == 0 {
            "temm1e-witness/tier0-v2"
        } else {
            "temm1e-witness/model-report-unresolved-v1"
        };
        if observation.schema_version != 1
            || observation.criterion_id != id
            || observation.workspace != criteria.workspace
            || observation.evaluator_version != version
            || result.predicate != *predicate
            || result.tier != tier
            || result.advisory != advisory
            || digest(&serde_json::to_string(observation).map_err(error)?) != entry.hash
        {
            return Err(error("observation integrity or predicate binding mismatch"));
        }
        let evaluator = if tier == 0 {
            EvaluatorKind::Deterministic
        } else {
            EvaluatorKind::ModelAssessment
        };
        // Current model verifier wiring supplies only a workspace/subtask label,
        // not resolved evidence_refs. Preserve its report but never promote it
        // into grounded assessment. A deterministic failure still dominates.
        let outcome = if tier > 0 {
            AssessmentOutcome::Inconclusive
        } else {
            match result.outcome {
                VerdictOutcome::Pass => AssessmentOutcome::Passed,
                VerdictOutcome::Fail => AssessmentOutcome::Failed,
                VerdictOutcome::Inconclusive => AssessmentOutcome::Inconclusive,
            }
        };
        hashes.insert(entry.hash.clone());
        requirements.push(Criterion {
            id: id.clone(),
            description: serde_json::to_string(predicate).map_err(error)?,
            evaluator,
            required: !advisory,
        });
        assessments.push(Assessment {
            criterion_id: id,
            outcome,
            evaluator,
            evaluator_version: version.into(),
            evidence_hashes: vec![entry.hash.clone()],
            reason: result.detail.clone(),
        });
    }
    Ok(assess_required(&requirements, &assessments, &hashes))
}

impl ExecutionJournal {
    pub(crate) async fn record_goal_assessment(
        &self,
        id: &str,
        session: &SessionContext,
        oath: &Oath,
        verdict: &Verdict,
        expected_revision: i64,
    ) -> Result<(), Temm1eError> {
        if expected_revision < 0
            || oath.root_goal_id != id
            || oath.session_id != session.session_id
            || verdict.subtask_id != oath.subtask_id
        {
            return Err(error("verification identity mismatch"));
        }
        let workspace = session.workspace_path.canonicalize().map_err(error)?;
        let mut tx = self.pool.begin().await.map_err(error)?;
        let (criteria_hash, document, objective): (String, String, String) = sqlx::query_as(
            "SELECT c.hash,c.document,g.objective FROM goal_criteria c JOIN goal_records g ON g.id=c.goal_id WHERE g.id=?")
            .bind(id).fetch_one(&mut *tx).await.map_err(error)?;
        let criteria = criteria_document(&document, &criteria_hash, id, &objective)?;
        if criteria.workspace != workspace
            || serde_json::to_string(&criteria.oath).map_err(error)?
                != serde_json::to_string(oath).map_err(error)?
        {
            return Err(error("verification does not match frozen criteria"));
        }
        let mut observations = Vec::new();
        if verdict.per_predicate.len() > 128 {
            return Err(error("too many observations"));
        }
        for (index, result) in verdict.per_predicate.iter().enumerate() {
            if result.detail.len() > 16 * 1024 {
                return Err(error(
                    "observation detail exceeds 16 KiB; no truncation implied",
                ));
            }
            let observation = EvaluatorObservation {
                schema_version: 1,
                kind: ObservationKind::EvaluatorReport,
                criterion_id: format!("{}:{index}", oath.sealed_hash),
                evaluator_version: if result.tier == 0 {
                    "temm1e-witness/tier0-v2".into()
                } else {
                    "temm1e-witness/model-report-unresolved-v1".into()
                },
                workspace: workspace.clone(),
                observed_at: verdict.rendered_at,
                result: result.clone(),
            };
            observations.push(HashedObservation {
                hash: digest(&serde_json::to_string(&observation).map_err(error)?),
                observation,
            });
        }
        let snapshot = GoalAssessment {
            schema_version: 1,
            criteria_hash: criteria_hash.clone(),
            coverage: CriteriaCoverage::Unverified,
            declared_outcome: declared_outcome(&criteria, &observations)?,
            observations,
        };
        let document = serde_json::to_string(&snapshot).map_err(error)?;
        if document.len() > MAX_DOCUMENT {
            return Err(error("assessment exceeds 4 MiB"));
        }
        let hash = digest(&document);
        let now = chrono::Utc::now().to_rfc3339();
        let principal = serde_json::to_string(&(&session.user_id, session.role)).map_err(error)?;
        let updated = sqlx::query("UPDATE goal_records SET revision=revision+1,updated_at=?
            WHERE id=? AND revision=? AND state='running' AND scope=? AND principal=?
            AND EXISTS(SELECT 1 FROM executions e WHERE e.id=goal_records.id AND e.state='running')
            AND NOT EXISTS(SELECT 1 FROM execution_conversations ec WHERE ec.execution_id=goal_records.id AND ec.epoch<>?)")
            .bind(&now).bind(id).bind(expected_revision).bind(Self::scope(session)?).bind(principal).bind(&session.session_id)
            .execute(&mut *tx).await.map_err(error)?;
        if updated.rows_affected() != 1 {
            return Err(error("stale, terminal or foreign assessment"));
        }
        sqlx::query("INSERT INTO goal_assessments(goal_id,criteria_hash,hash,document,created_at) VALUES(?,?,?,?,?)")
            .bind(id).bind(criteria_hash).bind(&hash).bind(document).bind(now).execute(&mut *tx).await.map_err(error)?;
        Self::goal_event(
            &mut tx,
            id,
            "declared_checks_assessed",
            serde_json::json!({"assessment_hash":hash,
            "declared_outcome":snapshot.declared_outcome,"coverage":"unverified"}),
        )
        .await?;
        tx.commit().await.map_err(error)?;
        Ok(())
    }

    /// Resolves bounded evaluator reports, not omitted raw artifact contents.
    pub async fn goal_assessment(
        &self,
        scope: &ConversationScope,
        id: &str,
    ) -> Result<Option<GoalAssessment>, Temm1eError> {
        let row: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT a.hash,a.document,c.hash,c.document,g.objective FROM goal_assessments a
            JOIN goal_records g ON g.id=a.goal_id JOIN goal_criteria c ON c.goal_id=g.id
            WHERE g.conversation_scope=? AND g.id=? AND a.criteria_hash=c.hash",
        )
        .bind(&scope.0)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(error)?;
        row.map(
            |(hash, document, criteria_hash, criteria_json, objective)| {
                if document.len() > MAX_DOCUMENT || digest(&document) != hash {
                    return Err(error("assessment integrity check failed"));
                }
                let saved: GoalAssessment = serde_json::from_str(&document).map_err(error)?;
                let criteria = criteria_document(&criteria_json, &criteria_hash, id, &objective)?;
                if saved.schema_version != 1
                    || saved.criteria_hash != criteria_hash
                    || declared_outcome(&criteria, &saved.observations)? != saved.declared_outcome
                {
                    return Err(error("invalid assessment document"));
                }
                Ok(saved)
            },
        )
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use temm1e_test_utils::{make_inbound_msg, make_session};
    use temm1e_witness::types::TierUsage;

    async fn fixture(
        directory: &std::path::Path,
        conditions: Vec<Predicate>,
    ) -> (
        Arc<ExecutionJournal>,
        SessionContext,
        ConversationScope,
        String,
        Oath,
        Verdict,
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
        let mut oath = Oath::draft("set", &id, &session.session_id, "original objective");
        oath.postconditions = conditions;
        oath.sealed_hash = hash_oath(&oath);
        journal
            .record_goal_criteria(&id, &session, &oath, 0)
            .await
            .unwrap();
        let per_predicate = oath
            .postconditions
            .iter()
            .map(|predicate| PredicateResult {
                predicate: predicate.clone(),
                tier: predicate.tier(),
                outcome: VerdictOutcome::Pass,
                detail: "fixture evaluator report".into(),
                latency_ms: 0,
                advisory: matches!(
                    predicate,
                    Predicate::AspectVerifier { advisory: true, .. }
                        | Predicate::AdversarialJudge { .. }
                ),
            })
            .collect();
        let verdict = Verdict {
            subtask_id: oath.subtask_id.clone(),
            rendered_at: chrono::Utc::now(),
            outcome: VerdictOutcome::Pass,
            per_predicate,
            tier_usage: TierUsage::default(),
            reason: "not authoritative".into(),
            cost_usd: 0.0,
            latency_ms: 0,
        };
        (journal, session, scope, id, oath, verdict)
    }
    fn file_check() -> Predicate {
        Predicate::FileExists {
            path: "fixture.txt".into(),
        }
    }

    #[tokio::test]
    async fn required_failure_cannot_be_overridden_by_reported_overall_or_advisory_pass() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath, mut verdict) = fixture(
            directory.path(),
            vec![
                file_check(),
                Predicate::AdversarialJudge {
                    rubric: "audit".into(),
                    evidence_refs: vec![],
                    advisory: false,
                },
            ],
        )
        .await;
        verdict.per_predicate[0].outcome = VerdictOutcome::Fail;
        journal
            .record_goal_assessment(&id, &session, &oath, &verdict, 1)
            .await
            .unwrap();
        let saved = journal.goal_assessment(&scope, &id).await.unwrap().unwrap();
        assert_eq!(saved.declared_outcome, AssessmentOutcome::Failed);
        journal
            .finish(&id, "reply_returned", &[], Some("DONE"))
            .await
            .unwrap();
        let status = journal.goal_status(&scope).await.unwrap();
        assert_eq!(
            status[0].state,
            temm1e_core::types::goal::GoalState::AwaitingEvidence
        );
        assert!(status[0].reason.contains("declared checks recorded"));
    }

    #[tokio::test]
    async fn unresolved_model_evidence_cannot_establish_a_grounded_assessment() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath, verdict) = fixture(
            directory.path(),
            vec![
                file_check(),
                Predicate::AspectVerifier {
                    rubric: "audit".into(),
                    evidence_refs: vec!["unresolved".into()],
                    advisory: false,
                },
            ],
        )
        .await;
        journal
            .record_goal_assessment(&id, &session, &oath, &verdict, 1)
            .await
            .unwrap();
        let saved = journal.goal_assessment(&scope, &id).await.unwrap().unwrap();
        assert_eq!(saved.declared_outcome, AssessmentOutcome::Inconclusive);
        assert_eq!(
            saved.observations[1].observation.result.outcome,
            VerdictOutcome::Pass,
            "preserve actual report without endorsing it"
        );
    }

    #[tokio::test]
    async fn mismatched_or_missing_observations_and_foreign_authority_roll_back() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath, verdict) =
            fixture(directory.path(), vec![file_check()]).await;
        for variant in 0..6 {
            let mut changed = verdict.clone();
            let mut caller = session.clone();
            match variant {
                0 => changed.per_predicate.clear(),
                1 => changed.per_predicate[0].advisory = true,
                2 => {
                    changed.per_predicate[0].predicate =
                        Predicate::DirectoryExists { path: ".".into() }
                }
                3 => changed.subtask_id = "other".into(),
                4 => caller.user_id = "bob".into(),
                _ => changed.per_predicate[0].tier = 1,
            }
            assert!(
                journal
                    .record_goal_assessment(&id, &caller, &oath, &changed, 1)
                    .await
                    .is_err(),
                "variant {variant}"
            );
        }
        assert_eq!(journal.goal_status(&scope).await.unwrap()[0].revision, 1);
        assert!(journal
            .goal_assessment(&scope, &id)
            .await
            .unwrap()
            .is_none());
        journal
            .record_goal_assessment(&id, &session, &oath, &verdict, 1)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn one_assessment_wins_reopens_scoped_and_rejects_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath, verdict) =
            fixture(directory.path(), vec![file_check()]).await;
        let (first, second) = tokio::join!(
            journal.record_goal_assessment(&id, &session, &oath, &verdict, 1),
            journal.record_goal_assessment(&id, &session, &oath, &verdict, 1)
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert!(journal
            .record_goal_assessment(&id, &session, &oath, &verdict, 2)
            .await
            .is_err());
        assert_eq!(journal.goal_status(&scope).await.unwrap()[0].revision, 2);
        let reopened = ExecutionJournal::open(&directory.path().join("executions.db"))
            .await
            .unwrap();
        assert_eq!(
            reopened
                .goal_assessment(&scope, &id)
                .await
                .unwrap()
                .unwrap()
                .declared_outcome,
            AssessmentOutcome::Passed
        );
        let foreign = ConversationScope::new(directory.path(), "test", "room", "bob").unwrap();
        assert!(reopened
            .goal_assessment(&foreign, &id)
            .await
            .unwrap()
            .is_none());
        sqlx::query("UPDATE goal_assessments SET document='{}' WHERE goal_id=?")
            .bind(&id)
            .execute(&journal.pool)
            .await
            .unwrap();
        assert!(reopened.goal_assessment(&scope, &id).await.is_err());
        assert!(reopened
            .goal_assessment(&foreign, &id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn terminal_or_oversized_assessment_preserves_prior_criteria_and_revision() {
        let directory = tempfile::tempdir().unwrap();
        let (journal, session, scope, id, oath, mut verdict) =
            fixture(directory.path(), vec![file_check()]).await;
        verdict.per_predicate[0].detail = "x".repeat(16 * 1024 + 1);
        assert!(journal
            .record_goal_assessment(&id, &session, &oath, &verdict, 1)
            .await
            .is_err());
        assert_eq!(journal.goal_status(&scope).await.unwrap()[0].revision, 1);
        verdict.per_predicate[0].detail = "within bound".into();
        journal.finish(&id, "interrupted", &[], None).await.unwrap();
        assert!(journal
            .record_goal_assessment(&id, &session, &oath, &verdict, 2)
            .await
            .is_err());
        assert!(journal
            .goal_assessment(&scope, &id)
            .await
            .unwrap()
            .is_none());
        assert!(journal.goal_criteria(&scope, &id).await.unwrap().is_some());
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use std::sync::Arc;
    use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory};
    use temm1e_witness::types::TierUsage;

    #[tokio::test]
    async fn inspection_limits_reports_and_escapes_terminal_control_sequences() {
        let directory = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            ExecutionJournal::open(&directory.path().join("executions.db"))
                .await
                .unwrap(),
        );
        let scope = ConversationScope::new(directory.path(), "test", "room", "alice").unwrap();
        let turn = journal.acquire_conversation(&scope).await.unwrap();
        let mut session = make_session();
        session.workspace_path = directory.path().into();
        session.channel = "test".into();
        session.chat_id = "room".into();
        session.user_id = "alice".into();
        session.session_id = turn.epoch().into();
        let id = journal
            .begin(&make_inbound_msg("original"), &session)
            .await
            .unwrap();
        let mut oath = Oath::draft("set", &id, &session.session_id, "original");
        oath.postconditions = vec![Predicate::DirectoryExists { path: ".".into() }; 13];
        oath.sealed_hash = hash_oath(&oath);
        journal
            .record_goal_criteria(&id, &session, &oath, 0)
            .await
            .unwrap();
        let verdict = Verdict {
            subtask_id: "set".into(),
            rendered_at: chrono::Utc::now(),
            outcome: VerdictOutcome::Pass,
            per_predicate: oath
                .postconditions
                .iter()
                .map(|predicate| PredicateResult {
                    predicate: predicate.clone(),
                    tier: 0,
                    outcome: VerdictOutcome::Pass,
                    detail: "\x1b]52;c;payload\x07\n".repeat(200),
                    advisory: false,
                    latency_ms: 0,
                })
                .collect(),
            tier_usage: TierUsage::default(),
            reason: "fixture".into(),
            cost_usd: 0.0,
            latency_ms: 0,
        };
        journal
            .record_goal_assessment(&id, &session, &oath, &verdict, 1)
            .await
            .unwrap();
        let report = crate::conversation::handle_owner_command(
            &journal,
            &scope,
            &MockMemory::new(),
            "legacy",
            &format!("/goal-assessment {id}"),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(report.contains("first 12"));
        assert!(!report.contains("Check 13:"));
        assert!(!report.contains('\x1b'));
        assert!(!report.contains('\x07'));
        assert!(report.len() < 12 * 1024);
        assert!(report.contains("Full request coverage: unverified"));
    }
}
