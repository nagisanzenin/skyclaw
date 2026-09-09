//! SkyGate server — axum-based HTTP server with health/status routes,
//! WebSocket upgrade support, and shared application state.

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use temm1e_agent::AgentRuntime;
use temm1e_core::types::config::GatewayConfig;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::Channel;
use tokio::net::TcpListener;
use tracing::info;

use crate::dashboard::{dashboard_config, dashboard_health, dashboard_page, dashboard_tasks};
use crate::health::{health_handler, ready_handler, status_handler};
use crate::identity::{oauth_callback_handler, OAuthIdentityManager};
use crate::session::SessionManager;

/// Shared application state accessible from all handlers.
pub struct AppState {
    pub channels: Vec<Arc<dyn Channel>>,
    pub agent: Arc<tokio::sync::RwLock<Option<Arc<AgentRuntime>>>>,
    pub config: GatewayConfig,
    pub sessions: SessionManager,
    pub identity: Option<Arc<OAuthIdentityManager>>,
}

/// The main SkyGate server.
pub struct SkyGate {
    state: Arc<AppState>,
}

impl SkyGate {
    /// Create a new SkyGate server.
    pub fn new(
        channels: Vec<Arc<dyn Channel>>,
        agent: Arc<AgentRuntime>,
        config: GatewayConfig,
    ) -> Self {
        let state = Arc::new(AppState {
            channels,
            agent: Arc::new(tokio::sync::RwLock::new(Some(agent))),
            config,
            sessions: SessionManager::new(),
            identity: None,
        });
        Self { state }
    }

    /// Create a new SkyGate server with an OAuth identity manager.
    pub fn with_identity(
        channels: Vec<Arc<dyn Channel>>,
        agent: Arc<AgentRuntime>,
        config: GatewayConfig,
        identity: OAuthIdentityManager,
    ) -> Self {
        let state = Arc::new(AppState {
            channels,
            agent: Arc::new(tokio::sync::RwLock::new(Some(agent))),
            config,
            sessions: SessionManager::new(),
            identity: Some(Arc::new(identity)),
        });
        Self { state }
    }

    /// Share the current runtime with onboarding and model-switch handlers.
    pub fn from_shared(
        channels: Vec<Arc<dyn Channel>>,
        agent: Arc<tokio::sync::RwLock<Option<Arc<AgentRuntime>>>>,
        config: GatewayConfig,
    ) -> Self {
        Self {
            state: Arc::new(AppState {
                channels,
                agent,
                config,
                sessions: SessionManager::new(),
                identity: None,
            }),
        }
    }

    /// Build the axum Router with all routes.
    fn build_router(&self) -> Router {
        let mut router = Router::new()
            .route("/health", get(health_handler))
            .route("/ready", get(ready_handler))
            .route("/status", get(status_handler))
            .route("/dashboard", get(dashboard_page))
            .route("/dashboard/api/health", get(dashboard_health))
            .route("/dashboard/api/tasks", get(dashboard_tasks))
            .route("/dashboard/api/config", get(dashboard_config))
            .with_state(self.state.clone());

        // Mount OAuth callback when identity is configured
        if let Some(ref identity) = self.state.identity {
            let auth_router = Router::new()
                .route("/auth/callback", get(oauth_callback_handler))
                .with_state(identity.clone());
            router = router.merge(auth_router);
        }

        router
    }

    /// Start the server, binding to the configured host and port.
    pub async fn start(&self) -> Result<(), Temm1eError> {
        self.start_with_shutdown(tokio_util::sync::CancellationToken::new())
            .await
    }

    /// Serve until the owning application begins shutdown.
    pub async fn start_with_shutdown(
        &self,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> Result<(), Temm1eError> {
        let addr = format!("{}:{}", self.state.config.host, self.state.config.port);
        info!(addr = %addr, "Starting SkyGate server");

        let listener = TcpListener::bind(&addr)
            .await
            .map_err(|e| Temm1eError::Internal(format!("Failed to bind to {}: {}", addr, e)))?;

        let router = self.build_router();

        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
            .map_err(|e| Temm1eError::Internal(format!("Server error: {}", e)))?;

        Ok(())
    }

    /// Get a reference to the shared application state.
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }

    /// Get a reference to the session manager.
    pub fn sessions(&self) -> &SessionManager {
        &self.state.sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use temm1e_test_utils::{MockMemory, MockProvider};
    use tower::ServiceExt;

    #[tokio::test]
    async fn onboarding_liveness_and_hot_runtime_readiness_are_distinct() {
        let shared = Arc::new(tokio::sync::RwLock::new(None));
        let gate = SkyGate::from_shared(vec![], shared.clone(), GatewayConfig::default());
        let router = gate.build_router();
        for (path, status) in [
            ("/health", StatusCode::OK),
            ("/ready", StatusCode::SERVICE_UNAVAILABLE),
            ("/dashboard", StatusCode::SERVICE_UNAVAILABLE),
            ("/dashboard/api/config", StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let response = router
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{path}");
        }
        *shared.write().await = Some(Arc::new(AgentRuntime::new(
            Arc::new(MockProvider::with_text("fixture")),
            Arc::new(MockMemory::new()),
            vec![],
            "hot-model".into(),
            None,
        )));
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/dashboard/api/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["model"], "hot-model");
        *shared.write().await = None;
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
