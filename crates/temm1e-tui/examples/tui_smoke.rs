//! Dev-only TUI wiring smoke test.
//!
//! Calls the exact same `spawn_agent` function that `launch_tui` calls,
//! but skips ratatui's terminal init so it runs headless. Used to
//! empirically verify the TUI parity gate per docs/RELEASE_PROTOCOL.md §7
//! without needing a real TTY.
//!
//! Usage:
//!   cargo run --example tui_smoke --features tui --release
//!
//! Expected stdout: registration logs for every feature wired into TUI.
//! The harness exits after 5 s so async init (Hive, Perpetuum, etc.) has
//! time to complete and emit its logs.

use std::time::Duration;

use temm1e_core::config::credentials;
use temm1e_core::types::config::Temm1eConfig;
use temm1e_core::types::message::InboundMessage;
use temm1e_tui::agent_bridge::{spawn_agent, AgentSetup};
use temm1e_tui::event::Event;
use tokio::sync::mpsc;

/// tui_smoke exit codes:
///   0  — all checks passed
///   2  — missing saved credentials
///   3  — spawn_agent returned Err
///   4  — agent did not respond within the response timeout
///   5  — agent response was empty
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Logs to stdout with info-level — same destination the parity grep
    // protocol expects to find registration anchors.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .init();

    eprintln!("=== TUI smoke: spawn_agent() direct-call (no ratatui) ===");

    // Load the same config TUI would use at launch.
    let config_dir = temm1e_core::config::data_dir();
    let config_path = config_dir.join("config.toml");
    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
    let config: Temm1eConfig = if config_str.is_empty() {
        Temm1eConfig::default()
    } else {
        toml::from_str(&config_str).unwrap_or_default()
    };

    // Resolve credentials from saved creds (mirrors TUI onboarding's fallback).
    let (provider_name, api_key, model) = match credentials::load_saved_credentials() {
        Some(t) => t,
        None => {
            eprintln!("[SMOKE FAIL] No saved credentials at ~/.temm1e/credentials.toml");
            std::process::exit(2);
        }
    };

    let setup = AgentSetup {
        provider_name: provider_name.clone(),
        api_key,
        model: model.clone(),
        base_url: None,
        config,
        mode: None,
    };

    // Event channel — spawn_agent pushes AgentResponseEvent via this.
    // We read them here to drive the exhaustive end-to-end test.
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<Event>();

    eprintln!(
        "[SMOKE] calling spawn_agent(provider={}, model={})",
        provider_name, model
    );
    let handle = match spawn_agent(setup, event_tx).await {
        Ok(h) => {
            eprintln!("[SMOKE] spawn_agent returned Ok — handle produced");
            h
        }
        Err(e) => {
            eprintln!("[SMOKE FAIL] spawn_agent returned Err: {}", e);
            std::process::exit(3);
        }
    };

    // Give async init (Hive SQLite, Perpetuum startup, Eigen-Tune DB
    // migration, etc.) 3 seconds to complete and emit their logs before
    // we drive the first message.
    eprintln!("[SMOKE] waiting 3s for async init logs to drain...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // ── EXHAUSTIVE TEST: drive a real user message through the agent ──
    // Prompt: defaults to "what can you do in 1 sentence?" but accepts a
    // --prompt "..." CLI arg so the harness can be driven through a UX
    // study or a regression battery.
    let args: Vec<String> = std::env::args().collect();
    let prompt_text = args
        .iter()
        .position(|a| a == "--prompt")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "what can you do in 1 sentence?".to_string());

    eprintln!("[SMOKE] sending: {prompt_text}");
    let t_send = std::time::Instant::now();
    let msg = InboundMessage {
        id: uuid::Uuid::new_v4().to_string(),
        chat_id: "tui".into(),
        user_id: "local".into(),
        username: Some("smoke".into()),
        channel: "tui".into(),
        text: Some(prompt_text.clone()),
        attachments: vec![],
        reply_to: None,
        timestamp: chrono::Utc::now(),
    };
    handle
        .inbound_tx
        .send(msg)
        .await
        .map_err(|e| format!("inbound send failed: {e}"))?;

    // Wait up to 180s for a response event (accommodates cold-start Perpetuum
    // + Chronos temporal_injection + consciousness pre_observe + classifier +
    // main reply on tier-1 providers).
    let timeout = Duration::from_secs(180);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut got_response = false;
    let mut streamed_deltas = 0usize;
    let mut response_text = String::new();
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, event_rx.recv()).await {
            Ok(Some(Event::TextLifecycle(
                temm1e_agent::agent_task_status::AgentTextEvent::Delta { .. },
            ))) => {
                streamed_deltas += 1;
            }
            Ok(Some(Event::AgentResponse(resp))) => {
                if resp.kind == temm1e_tui::event::ResponseKind::Failed {
                    eprintln!("[SMOKE FAIL] agent reported failure");
                    std::process::exit(5);
                }
                if resp.kind != temm1e_tui::event::ResponseKind::Final {
                    continue;
                }
                got_response = true;
                response_text = resp.message.text;
                let elapsed = t_send.elapsed();
                eprintln!(
                    "[SMOKE] wall={}.{}s usage: in={} out={} cost=${:.4}",
                    elapsed.as_secs(),
                    elapsed.subsec_millis() / 100,
                    resp.input_tokens,
                    resp.output_tokens,
                    resp.cost_usd
                );
                break;
            }
            Ok(Some(other)) => {
                // Diagnostic: log every non-response event so we can see what's firing.
                let tag = match &other {
                    Event::Terminal(_) => "Terminal",
                    Event::AgentStatus(_) => "AgentStatus",
                    Event::TextLifecycle(_) => "TextLifecycle",
                    Event::UserSubmit(_) => "UserSubmit",
                    Event::AgentResponse(_) => "AgentResponse",
                    _ => "Other",
                };
                eprintln!("[SMOKE-EVT] {tag}");
                continue;
            }
            Ok(None) => {
                eprintln!("[SMOKE FAIL] event channel closed before Complete event");
                std::process::exit(4);
            }
            Err(_) => {
                eprintln!("[SMOKE FAIL] timeout waiting for final agent response (180s)");
                std::process::exit(4);
            }
        }
    }

    if !got_response {
        eprintln!("[SMOKE FAIL] no response received within timeout");
        std::process::exit(4);
    }

    if response_text.trim().is_empty() {
        eprintln!("[SMOKE FAIL] agent response was empty");
        std::process::exit(5);
    }

    eprintln!("[SMOKE] AGENT RESPONDED ({} chars):", response_text.len());
    println!("{response_text}");
    if args.iter().any(|arg| arg == "--require-stream") && streamed_deltas == 0 {
        eprintln!("[SMOKE FAIL] no actual text deltas reached the TUI event channel");
        std::process::exit(6);
    }
    if let Some(expected) = args
        .iter()
        .position(|a| a == "--expect")
        .and_then(|i| args.get(i + 1))
    {
        if !response_text.contains(expected) {
            eprintln!("[SMOKE FAIL] final response did not contain the expected fixture value");
            std::process::exit(7);
        }
    }
    let shutdown = handle.shutdown(Duration::from_secs(6)).await;
    if !shutdown.loop_joined {
        eprintln!("[SMOKE FAIL] bridge did not finish its processing loop");
        std::process::exit(8);
    }
    if !prompt_text.trim_start().starts_with('/') {
        let journal = temm1e_agent::execution_journal::ExecutionJournal::open_profile().await?;
        let scope = temm1e_agent::conversation::ConversationScope::new(
            &config_dir.join("workspace"),
            "tui",
            "tui",
            "local-owner",
        )?;
        let records = journal.delivery_records(&scope).await?;
        if !records
            .first()
            .is_some_and(|record| record.state == "accepted_by_sink")
        {
            eprintln!("[SMOKE FAIL] final UI event acceptance was not durably recorded");
            std::process::exit(9);
        }
    }
    eprintln!("[SMOKE] processing_loop_joined={} background_drained={} (canceled hooks do not establish known usage/effects)", shutdown.loop_joined, shutdown.background_drained);
    eprintln!("[SMOKE] DONE — final response received; streamed_deltas={streamed_deltas}");
    Ok(())
}
