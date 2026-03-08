use serde::{Deserialize, Serialize};

/// Normalized inbound message from any channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub id: String,
    pub channel: String,
    pub chat_id: String,
    pub user_id: String,
    pub username: Option<String>,
    pub text: Option<String>,
    pub attachments: Vec<AttachmentRef>,
    pub reply_to: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Reference to a file attachment (platform-specific ID for lazy download)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentRef {
    pub file_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<usize>,
}

/// Outbound message to send via a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub chat_id: String,
    pub text: String,
    pub reply_to: Option<String>,
    pub parse_mode: Option<ParseMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParseMode {
    Markdown,
    Html,
    Plain,
}

/// Request to an AI model provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub system: Option<String>,
}

/// A single message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "image")]
    Image { media_type: String, data: String },
}

/// Tool definition for the AI model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Response from an AI model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub content: Vec<ContentPart>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

/// Streaming chunk from an AI model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    pub delta: Option<String>,
    pub tool_use: Option<ContentPart>,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_inbound_message() {
        let msg = InboundMessage {
            id: "msg-1".to_string(),
            channel: "telegram".to_string(),
            chat_id: "123".to_string(),
            user_id: "456".to_string(),
            username: Some("alice".to_string()),
            text: Some("Hello SkyClaw".to_string()),
            attachments: vec![AttachmentRef {
                file_id: "file-1".to_string(),
                file_name: Some("doc.pdf".to_string()),
                mime_type: Some("application/pdf".to_string()),
                size: Some(1024),
            }],
            reply_to: None,
            timestamp: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let restored: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, "msg-1");
        assert_eq!(restored.channel, "telegram");
        assert_eq!(restored.text.as_deref(), Some("Hello SkyClaw"));
        assert_eq!(restored.attachments.len(), 1);
        assert_eq!(
            restored.attachments[0].file_name.as_deref(),
            Some("doc.pdf")
        );
    }

    #[test]
    fn serde_roundtrip_completion_request() {
        let req = CompletionRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text("Hi".to_string()),
            }],
            tools: vec![ToolDefinition {
                name: "shell".to_string(),
                description: "Execute shell commands".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
            max_tokens: Some(4096),
            temperature: Some(0.7),
            system: Some("You are helpful".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let restored: CompletionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.model, "claude-sonnet-4-6");
        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.tools.len(), 1);
        assert_eq!(restored.max_tokens, Some(4096));
    }

    #[test]
    fn serde_content_part_text() {
        let part = ContentPart::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        let restored: ContentPart = serde_json::from_str(&json).unwrap();
        match restored {
            ContentPart::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn serde_content_part_tool_use() {
        let part = ContentPart::ToolUse {
            id: "tu-1".to_string(),
            name: "shell".to_string(),
            input: serde_json::json!({"command": "ls"}),
        };
        let json = serde_json::to_string(&part).unwrap();
        let restored: ContentPart = serde_json::from_str(&json).unwrap();
        match restored {
            ContentPart::ToolUse { id, name, input } => {
                assert_eq!(id, "tu-1");
                assert_eq!(name, "shell");
                assert_eq!(input["command"], "ls");
            }
            _ => panic!("expected ToolUse variant"),
        }
    }

    #[test]
    fn serde_content_part_image() {
        let part = ContentPart::Image {
            media_type: "image/jpeg".to_string(),
            data: "base64data".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"image\""));
        assert!(json.contains("\"media_type\":\"image/jpeg\""));
        let restored: ContentPart = serde_json::from_str(&json).unwrap();
        match restored {
            ContentPart::Image { media_type, data } => {
                assert_eq!(media_type, "image/jpeg");
                assert_eq!(data, "base64data");
            }
            _ => panic!("expected Image variant"),
        }
    }

    #[test]
    fn serde_role_lowercase() {
        let role = Role::Assistant;
        let json = serde_json::to_string(&role).unwrap();
        assert_eq!(json, "\"assistant\"");

        let restored: Role = serde_json::from_str("\"user\"").unwrap();
        assert!(matches!(restored, Role::User));
    }
}
