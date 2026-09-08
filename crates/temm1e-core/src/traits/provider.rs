use crate::types::error::Temm1eError;
use crate::types::message::{CompletionRequest, CompletionResponse, StreamChunk};
use async_trait::async_trait;
use futures::stream::BoxStream;

/// AI model provider trait. Implement this for each AI backend (Anthropic, OpenAI, etc.)
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name (e.g., "anthropic", "openai-compatible")
    fn name(&self) -> &str;

    /// Send a completion request and get a full response
    async fn complete(&self, request: CompletionRequest)
        -> Result<CompletionResponse, Temm1eError>;

    /// Optional real text streaming. Backends that do not implement it retain
    /// complete() semantics; callers must not synthesize token-by-token output.
    async fn complete_with_observer(
        &self,
        request: CompletionRequest,
        _observer: crate::streaming::TextObserver,
    ) -> Result<CompletionResponse, Temm1eError> {
        self.complete(request).await
    }

    /// Send a completion request and get a streaming response
    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, Temm1eError>>, Temm1eError>;

    /// Check if the provider is healthy and reachable
    async fn health_check(&self) -> Result<bool, Temm1eError>;

    /// List available models for this provider
    async fn list_models(&self) -> Result<Vec<String>, Temm1eError>;
}
