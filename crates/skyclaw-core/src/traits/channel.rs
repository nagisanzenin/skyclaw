use crate::types::error::SkyclawError;
use crate::types::file::{FileMetadata, OutboundFile, ReceivedFile};
use crate::types::message::{InboundMessage, OutboundMessage};
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;

/// Messaging channel trait. Implement for each platform (Telegram, Discord, etc.)
#[async_trait]
pub trait Channel: Send + Sync {
    /// Channel name (e.g., "telegram", "discord", "cli")
    fn name(&self) -> &str;

    /// Start the channel listener (connect to platform API)
    async fn start(&mut self) -> Result<(), SkyclawError>;

    /// Stop the channel listener gracefully
    async fn stop(&mut self) -> Result<(), SkyclawError>;

    /// Send a text message to a specific chat
    async fn send_message(&self, msg: OutboundMessage) -> Result<(), SkyclawError>;

    /// Get the file transfer capability for this channel (None if not supported)
    fn file_transfer(&self) -> Option<&dyn FileTransfer>;

    /// Check if a user is allowed to use this channel
    fn is_allowed(&self, user_id: &str) -> bool;
}

/// Bi-directional file transfer sub-trait. Every messaging channel should implement this.
#[async_trait]
pub trait FileTransfer: Send + Sync {
    /// Receive files attached to an inbound message
    async fn receive_file(&self, msg: &InboundMessage) -> Result<Vec<ReceivedFile>, SkyclawError>;

    /// Send a file to a user via the messaging platform
    async fn send_file(&self, chat_id: &str, file: OutboundFile) -> Result<(), SkyclawError>;

    /// Stream a large file with progress
    async fn send_file_stream(
        &self,
        chat_id: &str,
        stream: BoxStream<'_, Bytes>,
        metadata: FileMetadata,
    ) -> Result<(), SkyclawError>;

    /// Maximum file size this channel supports (in bytes)
    fn max_file_size(&self) -> usize;
}
