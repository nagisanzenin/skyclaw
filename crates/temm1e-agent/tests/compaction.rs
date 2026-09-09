use async_trait::async_trait;
use futures::stream::BoxStream;
use std::{sync::Arc, time::Duration};
use temm1e_agent::{compaction::*, execution_journal::ExecutionJournal, AgentRuntime};
use temm1e_core::{
    types::{error::Temm1eError, message::*},
    Provider,
};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, MockProvider};
struct Fixture {
    invalid: bool,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
}
#[async_trait]
impl Provider for Fixture {
    fn name(&self) -> &str {
        "fixture"
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        self.requests.lock().unwrap().push(request.clone());
        if request
            .system
            .as_deref()
            .is_some_and(|s| s.starts_with("Summarize the supplied transcript"))
        {
            if self.invalid {
                return Ok(MockProvider::with_text("not valid JSON").response);
            }
            let MessageContent::Text(text) = &request.messages[0].content else {
                panic!("missing source")
            };
            let source: serde_json::Value =
                serde_json::from_str(text.lines().skip(1).max_by_key(|line| line.len()).unwrap())
                    .unwrap();
            let summary = Summary {
                work_state: vec![HandoffItem {
                    text:
                        "Earlier assistant described ongoing work; this is not verified completion."
                            .into(),
                    citations: vec![Citation {
                        source_id: source["source_id"].as_str().unwrap().into(),
                        quote: source["quote_text"]
                            .as_str()
                            .unwrap()
                            .chars()
                            .take(64)
                            .collect(),
                    }],
                }],
                decisions: vec![],
                pending_work: vec![],
                uncertainties: vec![],
            };
            return Ok(MockProvider::with_text(&serde_json::to_string(&summary).unwrap()).response);
        }
        Ok(MockProvider::with_text("Ready to continue.").response)
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        Err(Temm1eError::Provider("unused".into()))
    }
    async fn health_check(&self) -> Result<bool, Temm1eError> {
        Ok(true)
    }
    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        Ok(vec![])
    }
}
#[tokio::test]
async fn runtime_commits_grounded_handoff_before_use_and_keeps_raw_history() {
    run(false, false).await;
}
#[tokio::test]
async fn invalid_summary_preserves_history_and_does_not_replace_durable_head() {
    run(true, false).await;
}
#[tokio::test]
async fn final_context_failure_does_not_commit_candidate_handoff() {
    run(false, true).await;
}
async fn run(invalid: bool, oversized: bool) {
    let dir = tempfile::tempdir().unwrap();
    let journal = Arc::new(
        ExecutionJournal::open(&dir.path().join("journal.db"))
            .await
            .unwrap(),
    );
    let provider = Arc::new(Fixture {
        invalid,
        requests: Default::default(),
    });
    let runtime = AgentRuntime::with_limits(
        provider.clone(),
        Arc::new(MockMemory::new()),
        vec![],
        "fixture".into(),
        Some(if oversized {
            "Oversized fixed instructions. ".repeat(4000)
        } else {
            "Test".into()
        }),
        200,
        12000,
        4,
        20,
        0.0,
    )
    .with_execution_journal(journal.clone())
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false);
    let mut session = make_session();
    session.workspace_path = dir.path().to_owned();
    for i in 0..8 {
        session.history.push(ChatMessage {
            role: Role::User,
            content: MessageContent::Text(format!(
                "Instruction {i}: do not publish; preserve amber."
            )),
        });
        session.history.push(ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text(
                "Earlier analysis of ongoing work; no verified result yet. ".repeat(80),
            ),
        });
    }
    let original = serde_json::to_string(&session.history).unwrap();
    let original_len = session.history.len();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        runtime.process_message(
            &make_inbound_msg("Correction: keep all original constraints and continue."),
            &mut session,
            None,
            None,
            None,
            None,
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_string(&session.history[..original_len]).unwrap(),
        original
    );
    let stored = journal.load_handoff(&session).await.unwrap();
    if invalid || oversized {
        assert!(result.is_err());
        assert!(stored.is_none());
    } else {
        let (_, usage) = result.unwrap();
        assert!(usage.api_calls >= 2);
        let (generation, handoff) = stored.unwrap();
        assert_eq!(generation, 1);
        assert!(handoff.matches_prefix(&session.history));
        // Reopen the database and exercise generation CAS before further turns.
        let reopened = ExecutionJournal::open(&dir.path().join("journal.db"))
            .await
            .unwrap();
        assert_eq!(reopened.load_handoff(&session).await.unwrap().unwrap().0, 1);
        assert!(reopened.save_handoff(&session, 0, &handoff).await.is_err());
        assert_eq!(reopened.load_handoff(&session).await.unwrap().unwrap().0, 1);
        let mut other = session.clone();
        other.user_id = "different-principal".into();
        assert!(reopened.load_handoff(&other).await.unwrap().is_none());
        for generation in 2..=3 {
            for i in 0..8 {
                session.history.push(ChatMessage {
                    role: Role::User,
                    content: MessageContent::Text(format!(
                        "Round {generation}, constraint {i}: do not publish."
                    )),
                });
                session.history.push(ChatMessage {
                    role: Role::Assistant,
                    content: MessageContent::Text(
                        "More analysis of ongoing work; no verified result yet. ".repeat(80),
                    ),
                });
            }
            tokio::time::timeout(
                Duration::from_secs(5),
                runtime.process_message(
                    &make_inbound_msg("Keep the earlier instructions and continue."),
                    &mut session,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            )
            .await
            .unwrap()
            .unwrap();
            let (actual, h) = journal.load_handoff(&session).await.unwrap().unwrap();
            assert_eq!(actual, generation);
            assert!(h.matches_prefix(&session.history));
            assert_eq!(
                serde_json::to_string(&session.history[..original_len]).unwrap(),
                original
            );
            assert!(serde_json::to_string(&h.pinned_messages())
                .unwrap()
                .contains("Instruction 0: do not publish"));
        }
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new().filename(dir.path().join("journal.db")),
        )
        .await
        .unwrap();
        let sources: i64 = sqlx::query_scalar("SELECT count(*) FROM context_source_messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        let (_, latest) = journal.load_handoff(&session).await.unwrap().unwrap();
        assert_eq!(
            sources as usize,
            latest.sources.len(),
            "raw prefix must not be copied once per generation"
        );
        let documents: Vec<String> = sqlx::query_scalar("SELECT document FROM context_handoffs")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            documents.iter().all(|d| {
                let value: serde_json::Value = serde_json::from_str(d).unwrap();
                value.get("sources").is_none() && value["source_ids"].is_array()
            }),
            "generation records hold source references, not repeated raw text"
        );
        // A later invalid generation must not overwrite a previously valid head.
        for i in 0..8 {
            session.history.push(ChatMessage {
                role: Role::User,
                content: MessageContent::Text(format!(
                    "Additional constraint {i}: retain prior requests."
                )),
            });
            session.history.push(ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Text(
                    "Further unverified analysis, without executed changes. ".repeat(80),
                ),
            });
        }
        let failed_runtime = AgentRuntime::with_limits(
            Arc::new(Fixture {
                invalid: true,
                requests: Default::default(),
            }),
            Arc::new(MockMemory::new()),
            vec![],
            "fixture".into(),
            Some("Test".into()),
            200,
            12000,
            4,
            20,
            0.0,
        )
        .with_execution_journal(journal.clone())
        .with_v2_optimizations(false)
        .with_self_audit_enabled(false);
        assert!(failed_runtime
            .process_message(
                &make_inbound_msg("Continue safely."),
                &mut session,
                None,
                None,
                None,
                None,
                None
            )
            .await
            .is_err());
        assert_eq!(journal.load_handoff(&session).await.unwrap().unwrap().0, 3);
        assert_eq!(
            serde_json::to_string(&session.history[..original_len]).unwrap(),
            original
        );
        let requests = provider.requests.lock().unwrap();
        assert!(requests
            .iter()
            .any(|r| r.tools.iter().any(|t| t.name == "context_recall")
                && serde_json::to_string(&r.messages)
                    .unwrap()
                    .contains("Instruction 0: do not publish")));
        assert!(requests.iter().any(|r| r
            .system_volatile
            .as_deref()
            .is_some_and(|s| s.contains("Context handoff v1"))));
    }
}

#[tokio::test]
async fn recall_reads_original_unicode_pages_and_rejects_wrong_scope() {
    use temm1e_core::{Tool, ToolContext, ToolInput};
    let dir = tempfile::tempdir().unwrap();
    let history = vec![ChatMessage {
        role: Role::Tool,
        content: MessageContent::Text("café 🐈 original result".into()),
    }];
    let h = Handoff::new(&history, Summary::default()).unwrap();
    let id = h.sources[0].id.clone();
    let recall = RecallTool {
        handoff: Arc::new(h),
        session_id: "s".into(),
        chat_id: "c".into(),
        workspace: dir.path().into(),
    };
    let mut ctx = ToolContext {
        user_id: "test-user".into(),
        role: temm1e_core::types::rbac::Role::Admin,
        channel: "cli".into(),
        workspace_path: dir.path().into(),
        session_id: "s".into(),
        chat_id: "c".into(),
        read_tracker: None,
    };
    let input = || ToolInput {
        name: "context_recall".into(),
        arguments: serde_json::json!({"source_id":id,"offset":5,"limit":1}),
    };
    let page = recall.execute(input(), &ctx).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&page.content).unwrap();
    assert_eq!(value["text"], "🐈");
    assert_eq!(value["next_offset"], 6);
    ctx.chat_id = "other".into();
    assert!(recall.execute(input(), &ctx).await.is_err());
}

#[tokio::test]
async fn inline_development_handoff_remains_readable_and_corrupt_sources_fail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let journal = ExecutionJournal::open(&path).await.unwrap();
    let mut session = make_session();
    session.workspace_path = dir.path().into();
    let history = vec![ChatMessage {
        role: Role::User,
        content: MessageContent::Text("Original constraint".into()),
    }];
    let handoff = Handoff::new(&history, Summary::default()).unwrap();
    journal.save_handoff(&session, 0, &handoff).await.unwrap();
    let pool =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(path))
            .await
            .unwrap();
    let inline = serde_json::to_string(&handoff).unwrap();
    sqlx::query("UPDATE context_handoffs SET document=? WHERE generation=1")
        .bind(inline)
        .execute(&pool)
        .await
        .unwrap();
    assert!(journal
        .load_handoff(&session)
        .await
        .unwrap()
        .unwrap()
        .1
        .matches_prefix(&history));
    journal.save_handoff(&session, 1, &handoff).await.unwrap();
    let altered = serde_json::to_string(&ChatMessage {
        role: Role::User,
        content: MessageContent::Text("Altered constraint".into()),
    })
    .unwrap();
    sqlx::query("UPDATE context_source_messages SET message=?")
        .bind(altered)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        journal.load_handoff(&session).await.is_err(),
        "tampered content must not silently load under its old source hash"
    );
}

#[tokio::test]
async fn canonical_restart_after_two_compactions_preserves_early_and_corrected_constraints() {
    use temm1e_agent::conversation::ConversationScope;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let scope = ConversationScope::new(dir.path(), "test", "chat", "local").unwrap();
    let journal = Arc::new(ExecutionJournal::open(&path).await.unwrap());
    let provider = Arc::new(Fixture {
        invalid: false,
        requests: Default::default(),
    });
    let make_runtime = |journal: Arc<ExecutionJournal>| {
        AgentRuntime::with_limits(
            provider.clone(),
            Arc::new(MockMemory::new()),
            vec![],
            "fixture".into(),
            Some("Test".into()),
            200,
            12000,
            4,
            20,
            0.0,
        )
        .with_execution_journal(journal)
        .with_v2_optimizations(false)
        .with_self_audit_enabled(false)
    };
    let mut history = vec![];
    for i in 0..125 {
        history.push(ChatMessage {
            role: Role::User,
            content: MessageContent::Text(if i == 0 {
                "Original: preserve archive_label maple17; do not publish.".into()
            } else {
                format!("Earlier turn {i}.")
            }),
        });
        history.push(ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text(
                "Earlier analysis; no verified result yet. ".repeat(if i > 116 { 110 } else { 1 }),
            ),
        });
    }
    journal
        .acquire_conversation(&scope)
        .await
        .unwrap()
        .commit(&history)
        .await
        .unwrap();
    let mut epoch = String::new();
    for round in 1..=2 {
        // Recreate both journal and runtime; the caller retains no chat history.
        let reopened = Arc::new(ExecutionJournal::open(&path).await.unwrap());
        let runtime = make_runtime(reopened.clone());
        let turn = reopened.acquire_conversation(&scope).await.unwrap();
        if epoch.is_empty() {
            epoch = turn.epoch().into();
        }
        assert_eq!(turn.epoch(), epoch);
        let mut session = make_session();
        session.session_id = turn.epoch().into();
        session.workspace_path = dir.path().to_owned();
        session.history = turn.history().to_vec();
        if round == 2 {
            for _ in 0..8 {
                session.history.push(ChatMessage {
                    role: Role::User,
                    content: MessageContent::Text(
                        "Correction: color amber, still do not publish.".into(),
                    ),
                });
                session.history.push(ChatMessage {
                    role: Role::Assistant,
                    content: MessageContent::Text(
                        "More ongoing analysis, not verified completion. ".repeat(110),
                    ),
                });
            }
        }
        runtime
            .process_message(
                &make_inbound_msg("Keep the original constraints and latest correction."),
                &mut session,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let (generation, handoff) = reopened.load_handoff(&session).await.unwrap().unwrap();
        assert_eq!(generation, round);
        assert!(handoff.matches_prefix(&session.history));
        assert!(serde_json::to_string(&handoff.pinned_messages())
            .unwrap()
            .contains("maple17"));
        if round == 2 {
            assert!(serde_json::to_string(&handoff.pinned_messages())
                .unwrap()
                .contains("color amber"));
        }
        turn.commit(&session.history).await.unwrap();
    }
    let reopened = Arc::new(ExecutionJournal::open(&path).await.unwrap());
    let turn = reopened.acquire_conversation(&scope).await.unwrap();
    assert!(turn.history().len() > 250);
    let restored = serde_json::to_string(turn.history()).unwrap();
    assert!(restored.contains("maple17"));
    assert!(restored.contains("color amber"));
    let history = turn.history().to_vec();
    turn.commit(&history).await.unwrap();
}
