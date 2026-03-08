use crate::types::error::SkyclawError;
use async_trait::async_trait;

/// Tunnel trait — secure external access (Cloudflare, Tailscale, ngrok, etc.)
#[async_trait]
pub trait Tunnel: Send + Sync {
    /// Start the tunnel and return the public URL
    async fn start(&mut self, local_port: u16) -> Result<String, SkyclawError>;

    /// Stop the tunnel
    async fn stop(&mut self) -> Result<(), SkyclawError>;

    /// Get the current public URL (None if not running)
    fn public_url(&self) -> Option<&str>;

    /// Tunnel provider name (e.g., "cloudflare", "ngrok")
    fn provider_name(&self) -> &str;
}
