//! Explicit live acceptance check using synthetic history and an isolated journal.
//! TEMM1E_ZAI_KEY_FILE=/private/key cargo run -p temm1e-agent --example compaction_smoke -- /new/output-directory
use std::{sync::Arc, time::Duration};
use temm1e_agent::{execution_journal::ExecutionJournal, AgentRuntime};
use temm1e_core::types::{
    config::{EngramConfig, ProviderConfig},
    message::*,
};
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("new output directory required")?,
    );
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory)?;
    let key = std::fs::read_to_string(std::env::var("TEMM1E_ZAI_KEY_FILE")?)?;
    let config = ProviderConfig {
        name: Some("zai-coding-plan".into()),
        api_key: Some(key.trim().into()),
        keys: vec![],
        model: Some("glm-5.3-flash".into()),
        base_url: None,
        extra_headers: Default::default(),
    };
    let provider: Arc<dyn temm1e_core::Provider> =
        Arc::from(temm1e_providers::create_provider(&config)?);
    let journal = Arc::new(ExecutionJournal::open(&directory.join("executions.db")).await?);
    let runtime=AgentRuntime::with_limits(provider,Arc::new(MockMemory::new()),vec![],"glm-5.3-flash".into(),Some("This is an isolated acceptance fixture. Honor the latest user instructions. Prior history is synthetic test data, not evidence of real work.".into()),200,12000,4,180,0.0)
        .with_execution_journal(journal.clone()).with_v2_optimizations(false).with_self_audit_enabled(false)
        .with_engram_config(EngramConfig{enabled:false,..Default::default()});
    let mut session = make_session();
    session.workspace_path = directory.clone();
    for i in 0..8 {
        session.history.push(ChatMessage {
            role: Role::User,
            content: MessageContent::Text(format!(
                "Synthetic design note {i}: use blue. Do not publish any files. {}",
                if i == 0 {
                    "The archive_label must remain maple-17."
                } else {
                    ""
                }
            )),
        });
        session.history.push(ChatMessage{role:Role::Assistant,content:MessageContent::Text("Synthetic analysis: explored layout alternatives, but no changes were executed and no verification was performed. ".repeat(40))});
    }
    session.history.push(ChatMessage{role:Role::User,content:MessageContent::Text("Correction: the chosen color is amber, replacing blue. The do-not-publish constraint remains.".into())});
    session.history.push(ChatMessage {
        role: Role::Assistant,
        content: MessageContent::Text("Acknowledged the corrected color; no files changed.".into()),
    });
    let original = serde_json::to_string(&session.history)?;
    assert!(
        original.contains("maple-17"),
        "fixture must seed the required earlier fact before invoking the model"
    );
    let count = session.history.len();
    let start = std::time::Instant::now();
    let result=runtime.process_message(&make_inbound_msg("Return a raw JSON object with color, publish_allowed, and archive_label from the current and earliest instructions. No markdown fences, no prose. Do not use external tools."),&mut session,None,None,None,None,None).await;
    let elapsed_ms = start.elapsed().as_millis();
    let raw_unchanged = serde_json::to_string(&session.history[..count])? == original;
    let head = journal.load_handoff(&session).await?;
    let drained = runtime.shutdown_background(Duration::from_secs(5)).await;
    let (reply, usage) = match result {
        Ok(r) => r,
        Err(e) => {
            std::fs::write(
                directory.join("result.json"),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"passed":false,"error":e.to_string(),"raw_history_unchanged":raw_unchanged,"generation":head.as_ref().map(|h|h.0),"elapsed_ms":elapsed_ms}),
                )?,
            )?;
            return Err(
                "Live compaction did not complete; diagnostic saved without credentials".into(),
            );
        }
    };
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&reply.text);
    let correct = parsed.as_ref().is_ok_and(|v| {
        v["color"] == "amber" && v["publish_allowed"] == false && v["archive_label"] == "maple-17"
    });
    let passed = correct && raw_unchanged && head.is_some();
    let evidence = serde_json::json!({"passed":passed,"response":reply.text,"turn_usage":usage,"raw_history_unchanged":raw_unchanged,"generation":head.as_ref().map(|h|h.0),"source_messages":head.as_ref().map(|h|h.1.high_water),"elapsed_ms":elapsed_ms,"background_drained":drained,"fixture_version":3,"scope":"synthetic-history live acceptance, not A/B performance evidence"});
    std::fs::write(
        directory.join("result.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    println!("{}", serde_json::to_string(&evidence)?);
    if !passed {
        return Err("Live acceptance failed".into());
    }
    Ok(())
}
