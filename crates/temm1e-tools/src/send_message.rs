//! Send message tool — sends a text message to the user during tool execution.
//! This allows the agent to send intermediate messages (progress updates,
//! periodic outputs, etc.) without waiting for the final reply.

use crate::channel_target::ChannelTarget;
use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use temm1e_core::types::error::Temm1eError;
use temm1e_core::types::message::OutboundMessage;
use temm1e_core::{Channel, Tool, ToolContext, ToolDeclarations, ToolInput, ToolOutput};

pub struct SendMessageTool {
    channel: ChannelTarget,
}

impl SendMessageTool {
    pub fn new(channel: Arc<dyn Channel>) -> Self {
        Self {
            channel: ChannelTarget::Single(channel),
        }
    }

    /// Server tools resolve from the active session's transport, preserving
    /// the configured heartbeat destination as the only explicit fallback.
    pub fn routed(
        channels: HashMap<String, Arc<dyn Channel>>,
        heartbeat: Option<Arc<dyn Channel>>,
    ) -> Self {
        Self {
            channel: ChannelTarget::Routed {
                channels,
                heartbeat,
            },
        }
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a text message to the user immediately during tool execution. \
         Use this when you need to send intermediate results, progress updates, \
         or periodic messages before your final reply. The message is delivered \
         instantly — you don't have to wait until the end of your response."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The message text to send"
                },
                "chat_id": {
                    "type": "string",
                    "description": "The chat ID to send to. Omit to send to the current conversation."
                }
            },
            "required": ["text"]
        })
    }

    fn declarations(&self) -> ToolDeclarations {
        ToolDeclarations {
            file_access: Vec::new(),
            network_access: Vec::new(),
            shell_access: false,
        }
    }

    async fn execute(
        &self,
        input: ToolInput,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, Temm1eError> {
        let text = input
            .arguments
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Temm1eError::Tool("Missing required parameter: text".into()))?;

        let chat_id = input
            .arguments
            .get("chat_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&ctx.chat_id);

        let outbound = OutboundMessage {
            chat_id: chat_id.to_string(),
            text: text.to_string(),
            reply_to: None,
            parse_mode: None,
        };

        match self
            .channel
            .resolve(&ctx.channel)?
            .send_message(outbound)
            .await
        {
            Ok(()) => Ok(ToolOutput {
                content: "Channel accepted the message; user receipt is unconfirmed".to_string(),
                is_error: false,
            }),
            Err(e) => Ok(ToolOutput {
                content: format!("Failed to send message: {}", e),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use temm1e_test_utils::MockChannel;

    #[tokio::test]
    async fn routes_equal_chat_ids_by_session_and_never_falls_back_for_unknown_transport() {
        let first = Arc::new(MockChannel::new("telegram"));
        let second = Arc::new(MockChannel::new("discord"));
        let channels: HashMap<String, Arc<dyn Channel>> = HashMap::from([
            ("telegram".into(), first.clone() as Arc<dyn Channel>),
            ("discord".into(), second.clone() as Arc<dyn Channel>),
        ]);
        let tool = SendMessageTool::routed(channels.clone(), Some(first.clone()));
        let mut ctx = ToolContext {
            user_id: "test-user".into(),
            role: temm1e_core::types::rbac::Role::Admin,
            channel: "discord".into(),
            chat_id: "42".into(),
            session_id: "epoch".into(),
            workspace_path: std::env::temp_dir(),
            read_tracker: None,
        };
        let input = || ToolInput {
            name: "send_message".into(),
            arguments: serde_json::json!({"text": "only discord"}),
        };
        assert!(!tool.execute(input(), &ctx).await.unwrap().is_error);
        assert_eq!(first.sent_count().await, 0);
        assert_eq!(second.sent_count().await, 1);
        assert_eq!(second.sent_messages.lock().await[0].chat_id, "42");
        ctx.channel = "unconfigured".into();
        assert!(tool.execute(input(), &ctx).await.is_err());
        assert_eq!(first.sent_count().await, 0);
        assert_eq!(second.sent_count().await, 1);
        // File sends use the identical resolver and reject unsupported routing
        // before reading a local file or falling back to another platform.
        let files = crate::SendFileTool::routed(channels, Some(first.clone()));
        let file_input = ToolInput {
            name: "send_file".into(),
            arguments: serde_json::json!({"path": "does-not-exist"}),
        };
        assert!(files.execute(file_input, &ctx).await.is_err());
        ctx.channel = "heartbeat".into();
        assert!(!tool.execute(input(), &ctx).await.unwrap().is_error);
        assert_eq!(first.sent_count().await, 1);
        assert_eq!(second.sent_count().await, 1);
    }
}
