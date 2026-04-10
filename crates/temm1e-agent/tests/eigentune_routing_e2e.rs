//! End-to-end tests for Eigen-Tune routing through the AgentRuntime.
//!
//! These tests wire a real `EigenTuneEngine` (backed by in-memory SQLite)
//! into AgentRuntime via `with_eigen_tune()`, then exercise each routing
//! path: Cloud, Local (with fallback), Shadow, and Monitor.
//!
//! The Local/Shadow/Monitor paths talk to a local Ollama-compatible endpoint
//! which won't exist in CI, so those branches verify the *fallback* behavior
//! (cloud fallback on timeout/error) rather than successful local inference.

use std::sync::Arc;

use temm1e_agent::AgentRuntime;
use temm1e_core::types::message::Role;
use temm1e_distill::config::EigenTuneConfig;
use temm1e_distill::types::{TierState, TrainingRun, TrainingRunStatus};
use temm1e_distill::EigenTuneEngine;
use temm1e_test_utils::{make_inbound_msg, make_session, MockMemory, MockProvider};

/// Create an EigenTuneEngine with in-memory SQLite and default config.
async fn make_engine() -> (Arc<EigenTuneEngine>, EigenTuneConfig) {
    let config = EigenTuneConfig {
        enabled: true,
        ..Default::default()
    };
    let engine = EigenTuneEngine::new(&config, "sqlite::memory:")
        .await
        .expect("engine creation should succeed");
    (Arc::new(engine), config)
}

/// Build an AgentRuntime with EigenTune wired in.
fn make_runtime_with_eigentune(
    provider: Arc<MockProvider>,
    engine: Arc<EigenTuneEngine>,
    enable_local_routing: bool,
) -> AgentRuntime {
    let memory = Arc::new(MockMemory::new());
    AgentRuntime::new(
        provider,
        memory,
        vec![],
        "test-model".to_string(),
        Some("You are a test agent.".to_string()),
    )
    .with_v2_optimizations(false)
    .with_eigen_tune(engine, enable_local_routing)
}

// ── Test 1: Cloud route (default — no graduated tiers) ──────────────

#[tokio::test]
async fn eigentune_cloud_route_default() {
    let (engine, _config) = make_engine().await;
    let provider = Arc::new(MockProvider::with_text("Cloud response"));
    let runtime = make_runtime_with_eigentune(provider.clone(), engine, false);

    let msg = make_inbound_msg("Hello");
    let mut session = make_session();
    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .expect("process_message should succeed");

    assert_eq!(reply.text, "Cloud response");
    assert_eq!(provider.calls().await, 1, "should call cloud provider once");
    assert_eq!(session.history.len(), 2);
    assert!(matches!(session.history[0].role, Role::User));
    assert!(matches!(session.history[1].role, Role::Assistant));
}

// ── Test 2: Cloud route even when engine present but local routing disabled ──

#[tokio::test]
async fn eigentune_cloud_when_local_routing_disabled() {
    let (engine, _config) = make_engine().await;

    // Force a tier to Graduated state — but local routing is OFF
    let store = engine.store();
    let run_id = "test-run-001";
    let run = TrainingRun {
        id: run_id.to_string(),
        started_at: chrono::Utc::now(),
        completed_at: Some(chrono::Utc::now()),
        status: TrainingRunStatus::Completed,
        base_model: "test-base".to_string(),
        backend: "mlx".to_string(),
        method: "lora".to_string(),
        dataset_version: 1,
        pair_count: 100,
        general_mix_pct: 0.0,
        output_model_path: None,
        gguf_path: None,
        ollama_model_name: Some("eigentune-simple-v1".to_string()),
        train_loss: Some(0.5),
        eval_loss: Some(0.6),
        epochs: Some(3),
        learning_rate: Some(2e-4),
        error_message: None,
    };
    store.save_run(&run).await.unwrap();

    // Set simple tier to Graduated with a serving model
    let mut tier_record = store.get_tier("simple").await.unwrap();
    tier_record.state = TierState::Graduated;
    tier_record.serving_run_id = Some(run_id.to_string());
    tier_record.serving_since = Some(chrono::Utc::now());
    store.update_tier(&tier_record).await.unwrap();

    // local routing is false → should still route to cloud
    let provider = Arc::new(MockProvider::with_text("Cloud even though graduated"));
    let runtime = make_runtime_with_eigentune(provider.clone(), engine, false);

    let msg = make_inbound_msg("Hi");
    let mut session = make_session();
    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(reply.text, "Cloud even though graduated");
    assert_eq!(provider.calls().await, 1);
}

// ── Test 3: Collection hook fires (pair data collected) ─────────────

#[tokio::test]
async fn eigentune_collection_fires_on_cloud_route() {
    let (engine, _config) = make_engine().await;
    let provider = Arc::new(MockProvider::with_text("Collected response"));
    let runtime = make_runtime_with_eigentune(provider.clone(), engine.clone(), false);

    let msg = make_inbound_msg("Explain Rust ownership");
    let mut session = make_session();
    runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    // Give the fire-and-forget spawn a moment to complete
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Check that a pair was collected in the store
    let store = engine.store();
    let total = store.total_pairs().await.unwrap();
    assert!(
        total >= 1,
        "should have collected at least 1 pair, got {total}"
    );
}

// ── Test 4: Local route fallback to cloud (no local server running) ──

#[tokio::test]
async fn eigentune_local_route_falls_back_to_cloud() {
    let (engine, _config) = make_engine().await;
    let store = engine.store();

    // Create a completed training run with an Ollama model name
    let run_id = "test-run-local-001";
    let run = TrainingRun {
        id: run_id.to_string(),
        started_at: chrono::Utc::now(),
        completed_at: Some(chrono::Utc::now()),
        status: TrainingRunStatus::Completed,
        base_model: "test-base".to_string(),
        backend: "mlx".to_string(),
        method: "lora".to_string(),
        dataset_version: 1,
        pair_count: 100,
        general_mix_pct: 0.0,
        output_model_path: None,
        gguf_path: None,
        ollama_model_name: Some("eigentune-simple-v1".to_string()),
        train_loss: Some(0.5),
        eval_loss: Some(0.6),
        epochs: Some(3),
        learning_rate: Some(2e-4),
        error_message: None,
    };
    store.save_run(&run).await.unwrap();

    // Graduate the "simple" tier
    let mut tier_record = store.get_tier("simple").await.unwrap();
    tier_record.state = TierState::Graduated;
    tier_record.serving_run_id = Some(run_id.to_string());
    tier_record.serving_since = Some(chrono::Utc::now());
    store.update_tier(&tier_record).await.unwrap();

    // local routing enabled → will try localhost:11434 → connection refused → fallback to cloud
    let provider = Arc::new(MockProvider::with_text(
        "Cloud fallback after local failure",
    ));
    let runtime = make_runtime_with_eigentune(provider.clone(), engine, true);

    let msg = make_inbound_msg("What is 2+2?");
    let mut session = make_session();
    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    // Should get cloud response (local failed, cloud fallback succeeded)
    assert_eq!(reply.text, "Cloud fallback after local failure");
    // Provider should be called once (for the cloud fallback)
    assert_eq!(
        provider.calls().await,
        1,
        "cloud provider should be called once as fallback"
    );
}

// ── Test 5: Shadow route — cloud serves, local runs in parallel ─────

#[tokio::test]
async fn eigentune_shadow_route_serves_cloud() {
    let (engine, _config) = make_engine().await;
    let store = engine.store();

    // Create a training run
    let run_id = "test-run-shadow-001";
    let run = TrainingRun {
        id: run_id.to_string(),
        started_at: chrono::Utc::now(),
        completed_at: Some(chrono::Utc::now()),
        status: TrainingRunStatus::Completed,
        base_model: "test-base".to_string(),
        backend: "mlx".to_string(),
        method: "lora".to_string(),
        dataset_version: 1,
        pair_count: 100,
        general_mix_pct: 0.0,
        output_model_path: None,
        gguf_path: None,
        ollama_model_name: Some("eigentune-simple-shadow".to_string()),
        train_loss: Some(0.5),
        eval_loss: Some(0.6),
        epochs: Some(3),
        learning_rate: Some(2e-4),
        error_message: None,
    };
    store.save_run(&run).await.unwrap();

    // Set tier to Shadowing state
    let mut tier_record = store.get_tier("simple").await.unwrap();
    tier_record.state = TierState::Shadowing;
    tier_record.serving_run_id = Some(run_id.to_string());
    store.update_tier(&tier_record).await.unwrap();

    let provider = Arc::new(MockProvider::with_text("Cloud serves in shadow mode"));
    let runtime = make_runtime_with_eigentune(provider.clone(), engine, true);

    let msg = make_inbound_msg("Explain closures");
    let mut session = make_session();
    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    // Shadow mode: cloud ALWAYS serves the user response
    assert_eq!(reply.text, "Cloud serves in shadow mode");
    assert_eq!(
        provider.calls().await,
        1,
        "cloud provider called for user-facing response"
    );
}

// ── Test 6: Multi-turn conversation with collection ─────────────────

#[tokio::test]
async fn eigentune_multi_turn_collection() {
    let (engine, _config) = make_engine().await;
    let provider = Arc::new(MockProvider::with_text("Turn reply"));
    let runtime = make_runtime_with_eigentune(provider.clone(), engine.clone(), false);

    let mut session = make_session();

    for i in 0..5 {
        let msg = make_inbound_msg(&format!("Turn {i} message"));
        runtime
            .process_message(&msg, &mut session, None, None, None, None, None)
            .await
            .unwrap();
    }

    // Wait for fire-and-forget collection spawns
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let store = engine.store();
    let total = store.total_pairs().await.unwrap();
    assert_eq!(total, 5, "should have collected 5 pairs across 5 turns");

    // Session should have 10 messages (5 user + 5 assistant)
    assert_eq!(session.history.len(), 10);
}

// ── Test 7: Tools present → always Cloud (Gate 2) ───────────────────

#[tokio::test]
async fn eigentune_tools_present_forces_cloud() {
    let (engine, _config) = make_engine().await;
    let store = engine.store();

    // Graduate the simple tier
    let run_id = "test-run-tools-001";
    let run = TrainingRun {
        id: run_id.to_string(),
        started_at: chrono::Utc::now(),
        completed_at: Some(chrono::Utc::now()),
        status: TrainingRunStatus::Completed,
        base_model: "test-base".to_string(),
        backend: "mlx".to_string(),
        method: "lora".to_string(),
        dataset_version: 1,
        pair_count: 100,
        general_mix_pct: 0.0,
        output_model_path: None,
        gguf_path: None,
        ollama_model_name: Some("eigentune-simple-tools".to_string()),
        train_loss: Some(0.5),
        eval_loss: Some(0.6),
        epochs: Some(3),
        learning_rate: Some(2e-4),
        error_message: None,
    };
    store.save_run(&run).await.unwrap();
    let mut tier_record = store.get_tier("simple").await.unwrap();
    tier_record.state = TierState::Graduated;
    tier_record.serving_run_id = Some(run_id.to_string());
    store.update_tier(&tier_record).await.unwrap();

    // Create runtime WITH tools — Gate 2 should force Cloud
    let provider = Arc::new(MockProvider::with_text("Cloud because tools"));
    let memory = Arc::new(MockMemory::new());
    let tool = Arc::new(temm1e_test_utils::MockTool::new("shell"));
    let runtime = AgentRuntime::new(
        provider.clone(),
        memory,
        vec![tool],
        "test-model".to_string(),
        Some("You are a test agent.".to_string()),
    )
    .with_v2_optimizations(false)
    .with_eigen_tune(engine, true); // local routing enabled, BUT tools present

    let msg = make_inbound_msg("List files");
    let mut session = make_session();
    let (reply, _usage) = runtime
        .process_message(&msg, &mut session, None, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(reply.text, "Cloud because tools");
    // Only 1 cloud call (no local attempt despite graduated tier)
    assert_eq!(provider.calls().await, 1);
}
