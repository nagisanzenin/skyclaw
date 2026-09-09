//! Goal achievement is independent of execution return and reply delivery.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const GOAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    Running,
    AwaitingEvidence,
    Recovering,
    Succeeded,
    Blocked,
    Cancelled,
    PausedQuota,
}
impl GoalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::AwaitingEvidence => "awaiting_evidence",
            Self::Recovering => "recovering",
            Self::Succeeded => "succeeded",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::PausedQuota => "paused_quota",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorKind {
    Deterministic,
    ModelAssessment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub description: String,
    pub evaluator: EvaluatorKind,
    pub required: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentOutcome {
    Passed,
    Failed,
    Inconclusive,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub criterion_id: String,
    pub outcome: AssessmentOutcome,
    pub evaluator: EvaluatorKind,
    pub evaluator_version: String,
    pub evidence_hashes: Vec<String>,
    pub reason: String,
}

/// This checks the declared criteria only; it cannot prove they cover the user's
/// intent. Evidence hashes must already have been resolved in this goal's scope.
/// Missing/ambiguous inputs never become a vacuous success. A model assessment
/// cannot substitute for a deterministic evaluator.
pub fn assess_required(
    criteria: &[Criterion],
    assessments: &[Assessment],
    available_hashes: &HashSet<String>,
) -> AssessmentOutcome {
    use AssessmentOutcome::*;
    if criteria.len() > 128 || assessments.len() > 128 || !criteria.iter().any(|c| c.required) {
        return Inconclusive;
    }
    let mut ids = HashSet::new();
    if criteria.iter().any(|c| {
        c.id.trim().is_empty() || c.description.trim().is_empty() || !ids.insert(c.id.as_str())
    }) {
        return Inconclusive;
    }
    let mut by_id = HashMap::new();
    if assessments.iter().any(|a| {
        !ids.contains(a.criterion_id.as_str()) || by_id.insert(a.criterion_id.as_str(), a).is_some()
    }) {
        return Inconclusive;
    }
    let mut incomplete = false;
    let mut failed = false;
    for criterion in criteria.iter().filter(|c| c.required) {
        let Some(assessment) = by_id.get(criterion.id.as_str()) else {
            incomplete = true;
            continue;
        };
        if assessment.evaluator != criterion.evaluator
            || assessment.evaluator_version.trim().is_empty()
            || assessment.reason.trim().is_empty()
            || assessment.evidence_hashes.is_empty()
            || assessment.evidence_hashes.iter().any(|h| {
                h.len() != 64
                    || !h.bytes().all(|b| b.is_ascii_hexdigit())
                    || !available_hashes.contains(h)
            })
        {
            incomplete = true;
            continue;
        }
        match assessment.outcome {
            Passed => {}
            Failed => failed = true,
            Inconclusive => incomplete = true,
        }
    }
    if failed {
        Failed
    } else if incomplete {
        Inconclusive
    } else {
        Passed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResultEvidence {
    pub schema_version: u32,
    pub operation_id: String,
    pub tool: String,
    pub is_error: bool,
    pub original_bytes: usize,
    pub original_sha256: String,
    pub truncated: bool,
    /// A bounded snapshot, not a path or a claim that the full task passed.
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_requires_actual_matching_evidence_for_every_required_criterion() {
        let hash = "a".repeat(64);
        let hashes = HashSet::from([hash.clone()]);
        let criterion = Criterion {
            id: "behavior".into(),
            description: "task test passes".into(),
            evaluator: EvaluatorKind::Deterministic,
            required: true,
        };
        let pass = Assessment {
            criterion_id: criterion.id.clone(),
            outcome: AssessmentOutcome::Passed,
            evaluator: EvaluatorKind::Deterministic,
            evaluator_version: "fixture-v1".into(),
            evidence_hashes: vec![hash],
            reason: "inspected actual task test result".into(),
        };
        let assess = |criteria: &[Criterion], assessments: &[Assessment]| {
            assess_required(criteria, assessments, &hashes)
        };
        assert_eq!(assess(&[], &[]), AssessmentOutcome::Inconclusive);
        assert_eq!(
            assess(std::slice::from_ref(&criterion), &[]),
            AssessmentOutcome::Inconclusive
        );
        assert_eq!(
            assess(
                std::slice::from_ref(&criterion),
                std::slice::from_ref(&pass)
            ),
            AssessmentOutcome::Passed
        );
        for invalid in [
            Assessment {
                evaluator: EvaluatorKind::ModelAssessment,
                ..pass.clone()
            },
            Assessment {
                evidence_hashes: vec![],
                ..pass.clone()
            },
            Assessment {
                evidence_hashes: vec!["b".repeat(64)],
                ..pass.clone()
            },
            Assessment {
                evaluator_version: String::new(),
                ..pass.clone()
            },
        ] {
            assert_eq!(
                assess(std::slice::from_ref(&criterion), &[invalid]),
                AssessmentOutcome::Inconclusive
            );
        }
        assert_eq!(
            assess(
                &[criterion.clone(), criterion.clone()],
                std::slice::from_ref(&pass)
            ),
            AssessmentOutcome::Inconclusive
        );
        assert_eq!(
            assess(
                std::slice::from_ref(&criterion),
                &[pass.clone(), pass.clone()]
            ),
            AssessmentOutcome::Inconclusive
        );
        let build = Criterion {
            id: "build".into(),
            ..criterion.clone()
        };
        let build_pass = Assessment {
            criterion_id: "build".into(),
            ..pass.clone()
        };
        let test_fail = Assessment {
            outcome: AssessmentOutcome::Failed,
            ..pass
        };
        assert_eq!(
            assess(&[build, criterion], &[build_pass, test_fail]),
            AssessmentOutcome::Failed
        );
    }
}
