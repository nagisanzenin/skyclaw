use async_trait::async_trait;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use temm1e_agent::AgentRuntime;
use temm1e_core::types::{
    error::Temm1eError,
    message::{ContentPart, MessageContent},
};
use temm1e_core::{Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, QueuedMockProvider};
use tokio_util::sync::CancellationToken;

struct SlowTool {
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<AtomicBool>,
    effect: Arc<AtomicBool>,
    dropped_signal: Arc<tokio::sync::Notify>,
    dropped_at: Arc<Mutex<Option<std::time::Instant>>>,
}
struct DropProbe {
    dropped: Arc<AtomicBool>,
    signal: Arc<tokio::sync::Notify>,
    at: Arc<Mutex<Option<std::time::Instant>>>,
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        *self.at.lock().unwrap() = Some(std::time::Instant::now());
        self.dropped.store(true, Ordering::SeqCst);
        self.signal.notify_one();
    }
}
#[async_trait]
impl Tool for SlowTool {
    fn name(&self) -> &str {
        "slow_fixture"
    }
    fn description(&self) -> &str {
        "Cancellation fixture"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: vec![],
            network_access: vec![],
            shell_access: false,
        }
    }
    async fn execute(&self, _: ToolInput, _: &ToolContext) -> Result<ToolOutput, Temm1eError> {
        let _probe = DropProbe {
            dropped: self.dropped.clone(),
            signal: self.dropped_signal.clone(),
            at: self.dropped_at.clone(),
        };
        self.started.notify_one();
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        self.effect.store(true, Ordering::SeqCst);
        Ok(ToolOutput {
            content: "finished".into(),
            is_error: false,
        })
    }
}

/// Keep three distinct budgets: admission/I/O, dropping the live tool future,
/// and committing durable uncertainty. The old whole-call 2s budget conflated
/// all three and could expire before a configured 1s task deadline was due.
async fn check_cancellation(mechanism: &'static str, delayed_admission: bool) {
    use std::time::{Duration, Instant};
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_signal = Arc::new(tokio::sync::Notify::new());
    let dropped_at = Arc::new(Mutex::new(None));
    let effect = Arc::new(AtomicBool::new(false));
    let tool = Arc::new(SlowTool {
        started: started.clone(),
        dropped: dropped.clone(),
        effect: effect.clone(),
        dropped_signal: dropped_signal.clone(),
        dropped_at: dropped_at.clone(),
    });
    let provider = Arc::new(QueuedMockProvider::with_responses(vec![
        QueuedMockProvider::tool_use_response("call", "slow_fixture", serde_json::json!({})),
    ]));
    let directory = tempfile::tempdir().unwrap();
    let journal = Arc::new(
        temm1e_agent::execution_journal::ExecutionJournal::open(
            &directory.path().join("executions.db"),
        )
        .await
        .unwrap(),
    );
    let release_lock = Arc::new(tokio::sync::Notify::new());
    let contention = if delayed_admission {
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(directory.path().join("executions.db")),
        )
        .await
        .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await
            .unwrap();
        let release = release_lock.clone();
        Some(tokio::spawn(async move {
            release.notified().await;
            tokio::time::sleep(Duration::from_millis(1200)).await;
            sqlx::query("COMMIT")
                .execute(&mut *connection)
                .await
                .unwrap();
        }))
    } else {
        None
    };
    let runtime = AgentRuntime::with_limits(
        provider,
        Arc::new(MockMemory::new()),
        vec![tool],
        "fixture-model".into(),
        Some("Test".into()),
        10,
        8000,
        6,
        if mechanism == "deadline" { 1 } else { 0 },
        0.0,
    )
    .with_execution_journal(journal.clone())
    .with_v2_optimizations(false)
    .with_self_audit_enabled(false);
    let token = CancellationToken::new();
    let flag = Arc::new(AtomicBool::new(false));
    let mut session = make_session();
    session.workspace_path = directory.path().to_owned();
    let message = make_inbound_msg("Run slow fixture");
    let start = Instant::now();
    let mut work = Box::pin(runtime.process_message(
        &message,
        &mut session,
        Some(flag.clone()),
        None,
        None,
        None,
        Some(token.clone()),
    ));
    release_lock.notify_one();
    tokio::select! {
        biased;
        _ = started.notified() => {},
        result = &mut work => panic!("{mechanism}: ended before fixture started after {:?}: {result:?}", start.elapsed()),
        _ = tokio::time::sleep(Duration::from_secs(15)) => panic!("{mechanism}: admission/tool-start I/O exceeded 15s"),
    }
    let admission = start.elapsed();
    let trigger = Instant::now();
    if mechanism == "legacy" {
        flag.store(true, Ordering::Relaxed);
    } else if mechanism == "token" {
        token.cancel();
    }
    // Deadline starts inside runtime after admission. It is due no later than
    // one second after tool start; allow the same 2s cancellation slack after it.
    let cancellation_limit = Duration::from_secs(if mechanism == "deadline" { 3 } else { 2 });
    let early_result = tokio::select! {
        biased;
        _ = dropped_signal.notified() => None,
        result = &mut work => Some(result),
        _ = tokio::time::sleep(cancellation_limit) => panic!("{mechanism}: live tool was not dropped within {cancellation_limit:?}; admission={admission:?}"),
    };
    assert!(
        dropped.load(Ordering::SeqCst),
        "{mechanism}: tool future must be dropped"
    );
    let drop_time = dropped_at.lock().unwrap().expect("drop timestamp");
    let cancellation = drop_time.saturating_duration_since(trigger);
    assert!(
        cancellation <= cancellation_limit,
        "{mechanism}: tool cancellation={cancellation:?}, admission={admission:?}"
    );
    let result = if let Some(result) = early_result {
        result
    } else {
        tokio::time::timeout(Duration::from_secs(10), &mut work).await
            .unwrap_or_else(|_| panic!("{mechanism}: tool dropped in {cancellation:?} but durable completion exceeded 10s"))
    };
    drop(work); // Release the mutable session borrow before inspecting history.
    let persistence = drop_time.elapsed();
    assert!(
        persistence <= Duration::from_secs(10),
        "{mechanism}: durable completion={persistence:?}"
    );
    let expected = if mechanism == "deadline" {
        "Task deadline reached"
    } else {
        "Task stopped"
    };
    assert!(result.unwrap_err().to_string().contains(expected));
    if let Some(contention) = contention {
        contention.await.unwrap();
    }
    let unfinished = journal.unfinished(&session).await.unwrap();
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].state, "interrupted");
    assert!(unfinished[0].checkpoint.contains("Outcome unknown"));
    assert!(!effect.load(Ordering::SeqCst));
    assert!(session
        .history
        .iter()
        .any(|message| match &message.content {
            MessageContent::Parts(parts) => parts.iter().any(|p| matches!(p,
            ContentPart::ToolResult { tool_use_id, content, is_error: true }
                if tool_use_id == "call" && content.contains("Outcome unknown"))),
            _ => false,
        }));
    eprintln!("{mechanism}: delayed_admission={delayed_admission}, admission={admission:?}, cancellation={cancellation:?}, persistence={persistence:?}");
}

#[tokio::test]
async fn token_drops_live_tool_before_durable_completion() {
    check_cancellation("token", false).await;
}
#[tokio::test]
async fn legacy_flag_drops_live_tool_before_durable_completion() {
    check_cancellation("legacy", false).await;
}
#[tokio::test]
async fn deadline_drops_live_tool_before_durable_completion() {
    check_cancellation("deadline", false).await;
}
#[tokio::test]
async fn sqlite_admission_contention_is_not_tool_cancellation_latency() {
    check_cancellation("deadline", true).await;
}
