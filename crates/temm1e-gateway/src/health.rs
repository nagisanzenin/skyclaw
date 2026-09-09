//! Health endpoint handler — returns JSON health/status information.

use std::sync::OnceLock;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

static START_TIME: OnceLock<Instant> = OnceLock::new();

/// Call this once at process startup to initialize the uptime clock.
pub fn init_start_time() {
    START_TIME.get_or_init(Instant::now);
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub provider: String,
    pub channels: Vec<String>,
    pub tools: Vec<String>,
    pub memory_backend: String,
}

/// Handler for GET /health
pub async fn health_handler() -> impl IntoResponse {
    let uptime = START_TIME.get().map(|t| t.elapsed().as_secs()).unwrap_or(0);
    let resp = HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_seconds: uptime,
    };
    (StatusCode::OK, Json(resp))
}

/// Handler for GET /status — provides detailed status including provider/channels/tools.
/// This version uses the shared AppState.
pub async fn status_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::server::AppState>>,
) -> impl IntoResponse {
    let channel_names: Vec<String> = state
        .channels
        .iter()
        .map(|c| c.name().to_string())
        .collect();

    let agent = state.agent.read().await.clone();
    let resp = StatusResponse {
        status: if agent.is_some() { "ok" } else { "onboarding" },
        version: env!("CARGO_PKG_VERSION"),
        provider: agent
            .as_ref()
            .map(|a| a.provider().name().to_owned())
            .unwrap_or_default(),
        channels: channel_names,
        tools: agent
            .as_ref()
            .map(|a| a.tools().iter().map(|t| t.name().to_owned()).collect())
            .unwrap_or_default(),
        memory_backend: agent
            .as_ref()
            .map(|a| a.memory().backend_name().to_owned())
            .unwrap_or_default(),
    };
    (StatusCode::OK, Json(resp))
}

/// Runtime readiness, distinct from process liveness. Does not probe provider connectivity.
pub async fn ready_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::server::AppState>>,
) -> impl IntoResponse {
    let ready = state.agent.read().await.is_some();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"status": if ready {"ready"} else {"onboarding"}})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_handler_returns_ok() {
        let response = health_handler().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn health_response_serializes_correctly() {
        let resp = HealthResponse {
            status: "ok",
            version: "1.0.0",
            uptime_seconds: 42,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["uptime_seconds"], 42);
    }

    #[test]
    fn status_response_serializes_correctly() {
        let resp = StatusResponse {
            status: "ok",
            version: "1.0.0",
            provider: "anthropic".to_string(),
            channels: vec!["telegram".to_string(), "cli".to_string()],
            tools: vec!["shell".to_string()],
            memory_backend: "sqlite".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["provider"], "anthropic");
        assert_eq!(json["channels"].as_array().unwrap().len(), 2);
        assert_eq!(json["memory_backend"], "sqlite");
    }
}
