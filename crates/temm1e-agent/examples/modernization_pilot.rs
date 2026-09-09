//! Shared A/B instrumentation. Copy this exact file to the pinned baseline;
//! no runtime changes are needed. Development pilot only, never a release gate.
use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use temm1e_core::{
    types::{
        config::ProviderConfig, error::Temm1eError, message::*, rbac, session::SessionContext,
    },
    Provider, Tool,
};

struct Meter {
    inner: Box<dyn Provider>,
    records: Mutex<Vec<serde_json::Value>>,
    calls: AtomicUsize,
    path: std::path::PathBuf,
}
impl Meter {
    fn record(&self, value: serde_json::Value) {
        let mut records = self.records.lock().unwrap();
        records.push(value);
        // Each completed call is persisted before returning to the runtime.
        std::fs::write(&self.path, serde_json::to_vec_pretty(&*records).unwrap()).unwrap();
    }
}
struct RequestGuard<'a> {
    meter: &'a Meter,
    call: usize,
    finished: bool,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.meter.record(
                serde_json::json!({"event":"request_cancelled","call":self.call,"usage":"unknown"}),
            );
        }
    }
}
#[async_trait]
impl Provider for Meter {
    fn name(&self) -> &str {
        self.inner.name()
    }
    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, Temm1eError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call >= 40 {
            return Err(Temm1eError::Provider(
                "Pilot request budget exhausted".into(),
            ));
        }
        // Equal-resource settings applied identically to all auxiliary calls.
        request.max_tokens = Some(request.max_tokens.unwrap_or(4096).min(4096));
        request.temperature = Some(1.0);
        let start = Instant::now();
        self.record(serde_json::json!({"event":"request_started","call":call,"request":request}));
        let mut guard = RequestGuard {
            meter: self,
            call,
            finished: false,
        };
        let result = self.inner.complete(request).await;
        match &result {
            Ok(response) => self.record(serde_json::json!({"event":"request_finished","call":call,"elapsed_ms":start.elapsed().as_millis(),"response":response})),
            Err(error) => self.record(serde_json::json!({"event":"request_failed","call":call,"elapsed_ms":start.elapsed().as_millis(),"error":error.to_string()})),
        }
        guard.finished = true;
        result
    }
    async fn stream(
        &self,
        _: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError> {
        Err(Temm1eError::Provider(
            "Pilot excludes streaming; do not silently leave calls unmetered".into(),
        ))
    }
    async fn health_check(&self) -> Result<bool, Temm1eError> {
        self.inner.health_check().await
    }
    async fn list_models(&self) -> Result<Vec<String>, Temm1eError> {
        self.inner.list_models().await
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: modernization_pilot INPUT_JSON WORKSPACE OUTPUT_JSON".into());
    }
    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let workspace = std::path::PathBuf::from(&args[2]).canonicalize()?;
    let output = std::path::PathBuf::from(&args[3]);
    let key = std::fs::read_to_string(std::env::var("TEMM1E_ZAI_KEY_FILE")?)?;
    // Existing compatible endpoint configuration supported by both versions.
    let config = ProviderConfig {
        name: Some("openai-compatible".into()),
        api_key: Some(key.trim().into()),
        keys: vec![],
        model: Some("glm-5.3-flash".into()),
        base_url: Some("https://api.z.ai/api/coding/paas/v4".into()),
        extra_headers: Default::default(),
    };
    let meter = Arc::new(Meter {
        inner: temm1e_providers::create_provider(&config)?,
        records: Mutex::new(vec![]),
        calls: AtomicUsize::new(0),
        path: output.with_extension("calls.json"),
    });
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(temm1e_tools::ShellTool::new()),
        Arc::new(temm1e_tools::FileReadTool::new()),
        Arc::new(temm1e_tools::FileWriteTool::new()),
    ];
    let runtime = temm1e_agent::AgentRuntime::with_limits(meter.clone(), Arc::new(temm1e_memory::SqliteMemory::new("sqlite::memory:").await?), tools, "glm-5.3-flash".into(), Some("Complete the user's task in the supplied workspace. Use tools to inspect, implement and execute checks. Report only observed results. Do not access paths outside this workspace. Do not install dependencies or use the network through tools.".into()), 20, 30000, 8, 240, 0.0).with_v2_optimizations(false);
    let mut session = SessionContext {
        session_id: "pilot".into(),
        channel: "pilot".into(),
        chat_id: "pilot".into(),
        user_id: "pilot".into(),
        role: rbac::Role::Admin,
        history: vec![],
        workspace_path: workspace,
        read_tracker: Arc::new(tokio::sync::RwLock::new(Default::default())),
    };
    let message = InboundMessage {
        id: input["id"].as_str().ok_or("missing id")?.into(),
        channel: "pilot".into(),
        chat_id: "pilot".into(),
        user_id: "pilot".into(),
        username: None,
        text: Some(input["prompt"].as_str().ok_or("missing prompt")?.into()),
        attachments: vec![],
        reply_to: None,
        timestamp: chrono::Utc::now(),
    };
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(245),
        runtime.process_message(&message, &mut session, None, None, None, None, None),
    )
    .await;
    let foreground_ms = start.elapsed().as_millis();
    let outcome = match result {
        Ok(Ok((reply, usage))) => {
            serde_json::json!({"status":"returned","reply":reply.text,"turn_usage":usage})
        }
        Ok(Err(error)) => serde_json::json!({"status":"error","error":error.to_string()}),
        Err(_) => {
            serde_json::json!({"status":"timeout","error":"245 second independent wall deadline"})
        }
    };
    // Identical bounded observation in both versions; no baseline runtime fix.
    let drain_start = Instant::now();
    let mut idle_since = None;
    let mut pending_requests;
    loop {
        pending_requests = {
            let records = meter.records.lock().unwrap();
            let started = records
                .iter()
                .filter(|r| r["event"] == "request_started")
                .count();
            let terminal = records
                .iter()
                .filter(|r| {
                    matches!(
                        r["event"].as_str(),
                        Some("request_finished" | "request_failed" | "request_cancelled")
                    )
                })
                .count();
            started.saturating_sub(terminal)
        };
        if pending_requests == 0 {
            let since = idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(1) {
                break;
            }
        } else {
            idle_since = None;
        }
        if drain_start.elapsed() >= Duration::from_secs(125) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let report = serde_json::json!({"task":input["id"],"foreground_ms":foreground_ms,"elapsed_ms":start.elapsed().as_millis(),"drain_ms":drain_start.elapsed().as_millis(),"pending_requests_at_report":pending_requests,"outcome":outcome,"history":session.history,"requests_started":meter.calls.load(Ordering::SeqCst).min(40)});
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "task={} status={} elapsed_ms={}",
        input["id"], report["outcome"]["status"], report["elapsed_ms"]
    );
    Ok(())
}
