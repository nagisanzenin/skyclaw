//! Real primitive checks must retain unknownness through composite logic.
use temm1e_witness::{
    ledger::Ledger,
    oath::hash_oath,
    predicates::{check_tier0, CheckContext},
    types::{Oath, Predicate, VerdictOutcome},
    Witness, WitnessError,
};
use VerdictOutcome::{Fail, Inconclusive, Pass};

fn primitive(outcome: VerdictOutcome) -> Predicate {
    match outcome {
        Pass => Predicate::FileExists {
            path: "present.txt".into(),
        },
        Fail => Predicate::FileExists {
            path: "missing.txt".into(),
        },
        Inconclusive => Predicate::ElapsedUnder {
            start_marker: "unset".into(),
            max_secs: 1,
        },
    }
}

#[tokio::test]
async fn real_composite_checks_follow_all_nine_three_valued_input_pairs() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("present.txt"), "fixture").unwrap();
    let ctx = CheckContext::new(directory.path());
    for left in [Pass, Fail, Inconclusive] {
        for right in [Pass, Fail, Inconclusive] {
            let and = if left == Fail || right == Fail {
                Fail
            } else if left == Pass && right == Pass {
                Pass
            } else {
                Inconclusive
            };
            let or = if left == Pass || right == Pass {
                Pass
            } else if left == Fail && right == Fail {
                Fail
            } else {
                Inconclusive
            };
            for (predicate, expected) in [
                (
                    Predicate::AllOf {
                        predicates: vec![primitive(left), primitive(right)],
                    },
                    and,
                ),
                (
                    Predicate::AnyOf {
                        predicates: vec![primitive(left), primitive(right)],
                    },
                    or,
                ),
            ] {
                let result = check_tier0(&predicate, &ctx).await.unwrap();
                assert_eq!(result.outcome, expected, "{predicate:?}");
                let inverse = Predicate::NotOf {
                    predicate: Box::new(predicate),
                };
                let expected = match expected {
                    Pass => Fail,
                    Fail => Pass,
                    Inconclusive => Inconclusive,
                };
                assert_eq!(check_tier0(&inverse, &ctx).await.unwrap().outcome, expected);
            }
        }
    }
}

#[tokio::test]
async fn empty_composites_and_their_negations_cannot_verify_a_requirement() {
    let directory = tempfile::tempdir().unwrap();
    let ctx = CheckContext::new(directory.path());
    for predicate in [
        Predicate::AllOf { predicates: vec![] },
        Predicate::AnyOf { predicates: vec![] },
    ] {
        assert_eq!(
            check_tier0(&predicate, &ctx).await.unwrap().outcome,
            Inconclusive
        );
        assert_eq!(
            check_tier0(
                &Predicate::NotOf {
                    predicate: Box::new(predicate)
                },
                &ctx
            )
            .await
            .unwrap()
            .outcome,
            Inconclusive
        );
    }
}

#[tokio::test]
async fn no_required_checks_never_produce_an_overall_pass() {
    let directory = tempfile::tempdir().unwrap();
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    );
    for conditions in [
        vec![],
        vec![Predicate::AspectVerifier {
            rubric: "audit".into(),
            evidence_refs: vec![],
            advisory: true,
        }],
    ] {
        let mut oath = Oath::draft("set", "goal", "session", "objective");
        oath.postconditions = conditions;
        oath.sealed_hash = hash_oath(&oath);
        assert_eq!(
            witness.verify_oath(&oath).await.unwrap().outcome,
            Inconclusive
        );
    }
}

#[tokio::test]
async fn changed_sealed_oath_is_rejected_before_verification() {
    let directory = tempfile::tempdir().unwrap();
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    );
    let mut oath = Oath::draft("set", "goal", "session", "original objective")
        .with_postcondition(Predicate::DirectoryExists { path: ".".into() });
    oath.sealed_hash = hash_oath(&oath);
    oath.goal = "tampered objective".into();
    assert!(matches!(
        witness.verify_oath(&oath).await,
        Err(WitnessError::TamperDetected { .. })
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn tampered_command_oath_starts_no_process() {
    let directory = tempfile::tempdir().unwrap();
    let witness = Witness::new(
        Ledger::open("sqlite::memory:").await.unwrap(),
        directory.path(),
    );
    let mut oath = Oath::draft("set", "goal", "session", "original objective")
        .with_postcondition(Predicate::DirectoryExists { path: ".".into() });
    oath.sealed_hash = hash_oath(&oath);
    oath.postconditions = vec![Predicate::CommandExits {
        cmd: "sh".into(),
        args: vec!["-c".into(), "printf tampered > marker.txt".into()],
        expected_code: 0,
        cwd: None,
        timeout_ms: 1000,
    }];
    assert!(matches!(
        witness.verify_oath(&oath).await,
        Err(WitnessError::TamperDetected { .. })
    ));
    assert!(!directory.path().join("marker.txt").exists());
}
