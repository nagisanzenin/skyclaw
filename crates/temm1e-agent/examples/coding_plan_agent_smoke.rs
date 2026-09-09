//! Explicit live smoke: a real agent turn, private temp workspace, independent artifact check.
use std::sync::Arc;
use temm1e_core::{
    types::{config::ProviderConfig, message::InboundMessage, rbac::Role, session::SessionContext},
    Tool,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let memory = Arc::new(temm1e_memory::SqliteMemory::new("sqlite::memory:").await?);
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(temm1e_tools::ShellTool::new())];
    let agent = temm1e_agent::AgentRuntime::with_limits(provider, memory, tools, "glm-5.3-flash".into(), Some("Work only in the provided workspace. Use the shell tool to perform and verify the requested file task. Do not merely describe commands.".into()), 10, 8000, 6, 90, 0.0).with_v2_optimizations(false);
    let dir = tempfile::tempdir()?;
    let mut session = SessionContext {
        session_id: "coding-plan-smoke".into(),
        channel: "test".into(),
        chat_id: "test".into(),
        user_id: "test".into(),
        role: Role::Admin,
        history: vec![],
        workspace_path: dir.path().into(),
        read_tracker: Arc::new(tokio::sync::RwLock::new(Default::default())),
    };
    let msg = InboundMessage { id: "smoke-1".into(), channel: "test".into(), chat_id: "test".into(), user_id: "test".into(), username: None, text: Some("Create answer.txt containing exactly 42 followed by a newline. Read it back with the shell to verify, then report the result.".into()), attachments: vec![], reply_to: None, timestamp: chrono::Utc::now() };
    let start = std::time::Instant::now();
    let (_, usage) = tokio::time::timeout(
        std::time::Duration::from_secs(100),
        agent.process_message(&msg, &mut session, None, None, None, None, None),
    )
    .await??;
    let contents = std::fs::read(dir.path().join("answer.txt"))?;
    if contents != b"42\n" {
        return Err("Independent artifact assertion failed".into());
    }
    println!(
        "artifact_verified=true elapsed_ms={} input_tokens={} output_tokens={} history_messages={}",
        start.elapsed().as_millis(),
        usage.input_tokens,
        usage.output_tokens,
        session.history.len()
    );
    Ok(())
}
