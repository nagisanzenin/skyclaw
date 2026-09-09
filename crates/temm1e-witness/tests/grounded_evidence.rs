//! Exercise evidence flow through the actual Witness dispatcher.
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use temm1e_witness::{
    ledger::Ledger,
    oath::hash_oath,
    types::{EvidenceKind, EvidenceSpec, Oath, Predicate, VerdictOutcome},
    witness::{LlmVerifierResponse, Tier1Verifier},
    Witness, WitnessError,
};

#[derive(Default)]
struct CapturingVerifier(Mutex<Vec<String>>);
#[async_trait]
impl Tier1Verifier for CapturingVerifier {
    async fn verify(
        &self,
        _: &str,
        _: &str,
        evidence: &str,
    ) -> Result<LlmVerifierResponse, WitnessError> {
        self.0.lock().unwrap().push(evidence.into());
        Ok(LlmVerifierResponse {
            verdict: "pass".into(),
            reason: "fixture assessment".into(),
        })
    }
}
fn oath(spec: Option<EvidenceSpec>) -> Oath {
    let mut oath = Oath::draft(
        "subtask",
        "goal",
        "session",
        "Verify the artifact's requested behavior",
    );
    oath.postconditions = vec![
        Predicate::DirectoryExists { path: ".".into() },
        Predicate::AspectVerifier {
            rubric: "Check the actual supplied artifact".into(),
            evidence_refs: vec!["artifact".into()],
            advisory: false,
        },
    ];
    if let Some(spec) = spec {
        oath.evidence_required.push(spec);
    }
    oath.sealed_hash = hash_oath(&oath);
    oath
}

#[tokio::test]
async fn missing_evidence_abstains_without_calling_a_model() {
    let directory = tempfile::tempdir().unwrap();
    let verifier = Arc::new(CapturingVerifier::default());
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    )
    .with_tier1(verifier.clone());
    let verdict = witness.verify_oath(&oath(None)).await.unwrap();
    assert_eq!(verdict.outcome, VerdictOutcome::Inconclusive);
    assert!(verifier.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn verifier_receives_actual_file_bytes_not_only_workspace_labels() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("artifact.txt"),
        "ACTUAL_ARTIFACT_SENTINEL_53",
    )
    .unwrap();
    let verifier = Arc::new(CapturingVerifier::default());
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    )
    .with_tier1(verifier.clone());
    let oath = oath(Some(EvidenceSpec {
        id: "artifact".into(),
        kind: EvidenceKind::File {
            path: "artifact.txt".into(),
        },
        description: "the actual file".into(),
    }));
    witness.verify_oath(&oath).await.unwrap();
    let calls = verifier.0.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].contains("ACTUAL_ARTIFACT_SENTINEL_53"),
        "model saw only: {}",
        calls[0]
    );
}

#[tokio::test]
async fn outside_workspace_and_unsupported_sources_abstain_without_model_calls() {
    let directory = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("private.txt"), "FOREIGN_SENTINEL").unwrap();
    let verifier = Arc::new(CapturingVerifier::default());
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    )
    .with_tier1(verifier.clone());
    for kind in [
        EvidenceKind::File {
            path: foreign.path().join("private.txt"),
        },
        EvidenceKind::CommandOutput {
            cmd: "never-launch".into(),
            args: vec![],
        },
    ] {
        let oath = oath(Some(EvidenceSpec {
            id: "artifact".into(),
            kind,
            description: "unavailable source".into(),
        }));
        assert_eq!(
            witness.verify_oath(&oath).await.unwrap().outcome,
            VerdictOutcome::Inconclusive
        );
    }
    assert!(verifier.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_files_and_ambiguous_references_never_reach_the_verifier() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("binary"), [0xff, 0xfe]).unwrap();
    std::fs::write(directory.path().join("large"), vec![b'x'; 16 * 1024 + 1]).unwrap();
    std::fs::write(directory.path().join("valid"), "valid source").unwrap();
    let verifier = Arc::new(CapturingVerifier::default());
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    )
    .with_tier1(verifier.clone());
    for path in ["binary", "large", ".", "missing"] {
        let oath = oath(Some(EvidenceSpec {
            id: "artifact".into(),
            kind: EvidenceKind::File { path: path.into() },
            description: "invalid".into(),
        }));
        assert_eq!(
            witness.verify_oath(&oath).await.unwrap().outcome,
            VerdictOutcome::Inconclusive
        );
    }
    let mut duplicate = oath(Some(EvidenceSpec {
        id: "artifact".into(),
        kind: EvidenceKind::File {
            path: "valid".into(),
        },
        description: "duplicate".into(),
    }));
    duplicate
        .evidence_required
        .push(duplicate.evidence_required[0].clone());
    duplicate.sealed_hash = hash_oath(&duplicate);
    assert_eq!(
        witness.verify_oath(&duplicate).await.unwrap().outcome,
        VerdictOutcome::Inconclusive
    );
    assert!(verifier.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn captured_report_is_immutable_content_not_a_later_file_lookup() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("artifact.txt"), "first version 🦀").unwrap();
    let verifier = Arc::new(CapturingVerifier::default());
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    )
    .with_tier1(verifier);
    let oath = oath(Some(EvidenceSpec {
        id: "artifact".into(),
        kind: EvidenceKind::File {
            path: "artifact.txt".into(),
        },
        description: "versioned bytes".into(),
    }));
    let report = witness.verify_oath_report(&oath).await.unwrap();
    assert_eq!(report.evidence.len(), 1);
    assert_eq!(report.evidence[0].predicate_index, 1);
    let snapshot = &report.evidence[0].snapshots[0];
    std::fs::remove_file(directory.path().join("artifact.txt")).unwrap();
    assert_eq!(snapshot.content, "first version 🦀");
    let workspace = directory.path().canonicalize().unwrap();
    snapshot.validate(&oath, &workspace).unwrap();
    let mut tampered = snapshot.clone();
    tampered.content = "replacement".into();
    assert!(tampered.validate(&oath, &workspace).is_err());
    let mut foreign = oath.clone();
    foreign.root_goal_id = "foreign".into();
    foreign.sealed_hash = hash_oath(&foreign);
    assert!(snapshot.validate(&foreign, &workspace).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn outside_symlinks_and_fifos_are_not_evidence_files() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("private"), "private").unwrap();
    symlink(
        foreign.path().join("private"),
        directory.path().join("link"),
    )
    .unwrap();
    let fifo = std::ffi::CString::new(directory.path().join("fifo").as_os_str().as_encoded_bytes())
        .unwrap();
    // SAFETY: fifo is a live NUL-terminated CString for an owned fixture path;
    // mkfifo only reads it during this call. No external data or process is involved.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let verifier = Arc::new(CapturingVerifier::default());
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    )
    .with_tier1(verifier.clone());
    for path in ["link", "fifo"] {
        let oath = oath(Some(EvidenceSpec {
            id: "artifact".into(),
            kind: EvidenceKind::File { path: path.into() },
            description: "not a scoped regular file".into(),
        }));
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            witness.verify_oath(&oath),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.outcome, VerdictOutcome::Inconclusive);
    }
    assert!(verifier.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn aggregate_bytes_and_repeated_reference_bounds_abstain_before_calling() {
    let directory = tempfile::tempdir().unwrap();
    let verifier = Arc::new(CapturingVerifier::default());
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    )
    .with_tier1(verifier.clone());
    let mut large = oath(None);
    let mut refs = Vec::new();
    for index in 0..3 {
        let name = format!("source{index}");
        refs.push(name.clone());
        std::fs::write(directory.path().join(&name), vec![b'x'; 16 * 1024]).unwrap();
        large.evidence_required.push(EvidenceSpec {
            id: name.clone(),
            kind: EvidenceKind::File { path: name.into() },
            description: "bounded file".into(),
        });
    }
    if let Predicate::AspectVerifier { evidence_refs, .. } = &mut large.postconditions[1] {
        *evidence_refs = refs;
    }
    large.sealed_hash = hash_oath(&large);
    assert_eq!(
        witness.verify_oath(&large).await.unwrap().outcome,
        VerdictOutcome::Inconclusive
    );
    if let Predicate::AspectVerifier { evidence_refs, .. } = &mut large.postconditions[1] {
        *evidence_refs = vec!["source0".into(), "source0".into()];
    }
    large.sealed_hash = hash_oath(&large);
    assert_eq!(
        witness.verify_oath(&large).await.unwrap().outcome,
        VerdictOutcome::Inconclusive
    );
    assert!(verifier.0.lock().unwrap().is_empty());
}
