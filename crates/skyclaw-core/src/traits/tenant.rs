use crate::types::error::SkyclawError;
use async_trait::async_trait;

/// Tenant trait — multi-tenancy isolation (stub for v0.1, designed for future)
#[async_trait]
pub trait Tenant: Send + Sync {
    /// Get tenant ID from a channel user
    async fn resolve_tenant(&self, channel: &str, user_id: &str) -> Result<TenantId, SkyclawError>;

    /// Get workspace path for a tenant
    fn workspace_path(&self, tenant_id: &TenantId) -> std::path::PathBuf;

    /// Check rate limits for a tenant
    async fn check_rate_limit(&self, tenant_id: &TenantId) -> Result<bool, SkyclawError>;
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TenantId(pub String);

impl TenantId {
    pub fn default_tenant() -> Self {
        Self("default".to_string())
    }
}
